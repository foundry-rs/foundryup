use eyre::{Result, WrapErr, bail};
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path};

pub(crate) struct Downloader {
    client: reqwest::Client,
}

impl Downloader {
    pub(crate) fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("foundryup/", env!("CARGO_PKG_VERSION")))
            .build()
            .wrap_err("failed to create HTTP client")?;
        Ok(Self { client })
    }

    pub(crate) async fn download_to_file(&self, url: &str, path: &Path) -> Result<()> {
        let response =
            self.client.get(url).send().await.wrap_err_with(|| format!("failed to GET {url}"))?;

        if !response.status().is_success() {
            bail!("failed to download {url}: HTTP {}", response.status());
        }

        let total_size = response.content_length();

        let pb = match total_size {
            Some(size) => {
                let pb = ProgressBar::new(size);
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template(
                            "{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
                        )
                        .unwrap()
                        .progress_chars("#>-"),
                );
                pb
            }
            None => {
                let pb = ProgressBar::new_spinner();
                pb.set_style(
                    ProgressStyle::default_spinner().template("{spinner:.green} {bytes}").unwrap(),
                );
                pb
            }
        };

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(path)?;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.wrap_err("error reading response")?;
            file.write_all(&chunk)?;
            pb.inc(chunk.len() as u64);
        }

        pb.finish_and_clear();
        Ok(())
    }

    pub(crate) async fn download_to_string(&self, url: &str) -> Result<String> {
        let response =
            self.client.get(url).send().await.wrap_err_with(|| format!("failed to GET {url}"))?;

        if !response.status().is_success() {
            bail!("failed to download {url}: HTTP {}", response.status());
        }

        response.text().await.wrap_err("failed to read response body")
    }
}

pub(crate) fn compute_sha256(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).wrap_err("failed to open file for hashing")?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    std::fs::create_dir_all(dest_dir)?;
    archive.unpack(dest_dir).wrap_err("failed to extract tar.gz archive")?;
    Ok(())
}

pub(crate) fn extract_zip(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    std::fs::create_dir_all(dest_dir)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = match file.enclosed_name() {
            Some(path) => dest_dir.join(path),
            None => continue,
        };

        if file.is_dir() {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut outfile = std::fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = file.unix_mode() {
                std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode))?;
            }
        }
    }

    Ok(())
}
