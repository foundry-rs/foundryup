use eyre::{Result, WrapErr, bail};
use fs_err as fs;
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path};

/// Number of retries (after the initial attempt) for transient HTTP failures.
const MAX_RETRIES: u32 = 5;

/// Transient HTTP statuses that may recover on retry (e.g. GitHub rate limiting
/// or temporary outages). Other errors (e.g. 404) are treated as permanent.
fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 403 | 408 | 429 | 500 | 502 | 503 | 504)
}

/// Retry scope matching every GitHub host foundryup talks to, including the
/// CDN hosts that release downloads redirect to.
struct GitHubHosts;

impl PartialEq<&str> for GitHubHosts {
    fn eq(&self, host: &&str) -> bool {
        // Require a `.` boundary so lookalikes like `notgithub.com` don't match.
        let host = host.trim_end_matches('.');
        ["github.com", "githubusercontent.com"].iter().any(|domain| {
            host == *domain || host.strip_suffix(domain).is_some_and(|prefix| prefix.ends_with('.'))
        })
    }
}

pub(crate) struct Downloader {
    client: reqwest::Client,
}

impl Downloader {
    pub(crate) fn new() -> Result<Self> {
        // `no_budget` disables reqwest's default token budget, which would
        // otherwise block retries on a CLI that issues only a few requests.
        let retry = reqwest::retry::for_host(GitHubHosts)
            .no_budget()
            .max_retries_per_request(MAX_RETRIES)
            .classify_fn(|req_rep| {
                if req_rep.error().is_some() || req_rep.status().is_some_and(is_retryable_status) {
                    req_rep.retryable()
                } else {
                    req_rep.success()
                }
            });

        let client = reqwest::Client::builder()
            .user_agent(concat!("foundryup/", env!("CARGO_PKG_VERSION")))
            .retry(retry)
            .build()
            .wrap_err("failed to create HTTP client")?;
        Ok(Self { client })
    }

    async fn send(&self, url: &str) -> Result<reqwest::Response> {
        let response =
            self.client.get(url).send().await.wrap_err_with(|| format!("failed to GET {url}"))?;
        if !response.status().is_success() {
            bail!("failed to download {url}: HTTP {}", response.status());
        }
        Ok(response)
    }

    pub(crate) async fn download_to_file(&self, url: &str, path: &Path) -> Result<()> {
        let response = self.send(url).await?;

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
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(path)?;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.wrap_err("failed to read response chunk")?;
            file.write_all(&chunk)?;
            pb.inc(chunk.len() as u64);
        }

        pb.finish_and_clear();
        Ok(())
    }

    pub(crate) async fn download_to_string(&self, url: &str) -> Result<String> {
        let response = self.send(url).await?;
        response.text().await.wrap_err("failed to read response body")
    }
}

pub(crate) fn compute_sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let file = fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    fs::create_dir_all(dest_dir)?;
    archive.unpack(dest_dir)?;
    Ok(())
}

pub(crate) fn extract_zip(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let file = fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    fs::create_dir_all(dest_dir)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = match file.enclosed_name() {
            Some(path) => dest_dir.join(path),
            None => continue,
        };

        if file.is_dir() {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() {
                fs::create_dir_all(p)?;
            }
            let mut outfile = fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = file.unix_mode() {
                fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode))?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_status_classification() {
        for code in [403, 408, 429, 500, 502, 503, 504] {
            assert!(is_retryable_status(reqwest::StatusCode::from_u16(code).unwrap()));
        }
        for code in [200, 301, 400, 401, 404, 410] {
            assert!(!is_retryable_status(reqwest::StatusCode::from_u16(code).unwrap()));
        }
    }

    #[test]
    fn github_hosts_scope_matches_github_cdns() {
        assert!(GitHubHosts == "github.com");
        assert!(GitHubHosts == "api.github.com");
        assert!(GitHubHosts == "raw.githubusercontent.com");
        assert!(GitHubHosts == "objects.githubusercontent.com");
        assert!(GitHubHosts != "example.com");
        assert!(GitHubHosts != "notgithub.com");
        assert!(GitHubHosts != "evilgithubusercontent.com");
        assert!(GitHubHosts != "github.com.evil.com");
    }
}
