use crate::{
    cli::{Cli, Network},
    config::Config,
    download::{Downloader, compute_sha256, extract_tar_gz, extract_zip},
    platform::{Platform, Target},
    say, warn,
};
use eyre::{Result, WrapErr, bail};
use fs_err as fs;
use std::{collections::HashMap, path::Path};

pub(crate) async fn run(config: &Config, args: &Cli) -> Result<()> {
    config.ensure_dirs()?;

    if let Some(ref local_path) = args.path {
        return install_from_local(config, local_path, args).await;
    }

    let network = args.network;
    let repo = args.repo.as_deref().unwrap_or_else(|| config.repo(network));

    let should_build = args.branch.is_some() || args.pr.is_some() || args.commit.is_some();
    let is_foundry_repo = repo == crate::config::FOUNDRY_REPO;
    let is_tempo = network == Some(Network::Tempo);

    if is_foundry_repo && !should_build {
        install_prebuilt(config, args).await
    } else if is_tempo && !should_build {
        install_tempo_prebuilt(config, args).await
    } else {
        install_from_source(config, repo, args).await
    }
}

async fn install_prebuilt(config: &Config, args: &Cli) -> Result<()> {
    let version = normalize_version(args.version.as_deref().unwrap_or("stable"));
    let tag = version_to_tag(&version);

    say(&format!("installing foundry (version {version}, tag {tag})"));

    let target = Target::detect(args.platform.as_deref(), args.arch.as_deref())?;
    let downloader = Downloader::new()?;

    let release_url =
        format!("https://github.com/{}/releases/download/{tag}/", crate::config::FOUNDRY_REPO);

    let bins = config.bins(args.network);

    let hashes = if !args.force {
        fetch_and_verify_attestation(config, &downloader, &release_url, &version, &target, bins)
            .await?
    } else {
        say("skipped SHA verification due to --force flag");
        None
    };

    download_and_extract(config, &downloader, &release_url, &version, &tag, &target).await?;

    if let Some(ref hashes) = hashes {
        verify_installed_binaries(config, &tag, bins, hashes)?;
    }

    download_manpages(config, &downloader, &release_url, &version).await;

    use_version(config, &tag)?;
    say("done!");

    Ok(())
}

async fn install_tempo_prebuilt(config: &Config, args: &Cli) -> Result<()> {
    let version = args.version.as_deref().unwrap_or("nightly");
    let tag = version.to_string();

    say(&format!("installing tempo-foundry (version {version}, tag {tag})"));

    let target = Target::detect(args.platform.as_deref(), args.arch.as_deref())?;
    let downloader = Downloader::new()?;

    let release_url =
        format!("https://github.com/{}/releases/download/{tag}/", crate::config::TEMPO_REPO);

    download_and_extract_tempo(config, &downloader, &release_url, &target, &tag).await?;
    download_manpages(config, &downloader, &release_url, "nightly").await;

    use_version(config, &tag)?;
    say("done!");

    Ok(())
}

async fn install_from_local(config: &Config, local_path: &Path, args: &Cli) -> Result<()> {
    if args.repo.is_some() || args.branch.is_some() || args.version.is_some() {
        warn("--branch, --install, --use, and --repo arguments are ignored during local install");
    }

    say(&format!("installing from {}", local_path.display()));

    let mut cmd = tokio::process::Command::new("cargo");
    cmd.arg("build").arg("--bins").arg("--release").current_dir(local_path);

    if let Some(jobs) = args.jobs {
        cmd.arg("--jobs").arg(jobs.to_string());
    }

    let status = cmd.status().await.wrap_err("failed to run cargo build")?;
    if !status.success() {
        bail!("cargo build failed");
    }

    let bins = config.bins(args.network);
    for bin in bins {
        let src = local_path.join("target/release").join(bin_name(bin));
        let dest = config.bin_path(bin);

        if dest.exists() {
            fs::remove_file(&dest)?;
        }

        #[cfg(unix)]
        std::os::unix::fs::symlink(&src, &dest)?;
        #[cfg(windows)]
        fs::copy(&src, &dest)?;
    }

    say("done");
    Ok(())
}

async fn install_from_source(config: &Config, repo: &str, args: &Cli) -> Result<()> {
    let branch = if let Some(pr) = args.pr {
        format!("refs/pull/{pr}/head")
    } else {
        args.branch.clone().unwrap_or_else(|| "master".to_string())
    };

    let repo_path = config.repo_dir(repo);
    let author = repo.split('/').next().unwrap_or(repo);

    if !repo_path.exists() {
        let author_dir = config.foundry_dir.join(author);
        fs::create_dir_all(&author_dir)?;

        say(&format!("cloning {repo}..."));
        let status = tokio::process::Command::new("git")
            .args(["clone", &format!("https://github.com/{repo}")])
            .current_dir(&author_dir)
            .status()
            .await?;
        if !status.success() {
            bail!("git clone failed");
        }
    }

    say(&format!("fetching {branch}..."));
    let status = tokio::process::Command::new("git")
        .args(["fetch", "origin", &format!("{branch}:remotes/origin/{branch}")])
        .current_dir(&repo_path)
        .status()
        .await?;
    if !status.success() {
        bail!("git fetch failed");
    }

    let status = tokio::process::Command::new("git")
        .args(["checkout", &format!("origin/{branch}")])
        .current_dir(&repo_path)
        .status()
        .await?;
    if !status.success() {
        bail!("git checkout failed");
    }

    if let Some(ref commit) = args.commit {
        let status = tokio::process::Command::new("git")
            .args(["checkout", commit])
            .current_dir(&repo_path)
            .status()
            .await?;
        if !status.success() {
            bail!("git checkout commit failed");
        }
    }

    let version = if let Some(ref commit) = args.commit {
        format!("{author}-commit-{commit}")
    } else if let Some(pr) = args.pr {
        format!("{author}-pr-{pr}")
    } else {
        let normalized_branch = branch.replace('/', "-");
        format!("{author}-branch-{normalized_branch}")
    };

    say(&format!("installing version {version}"));

    let mut cmd = tokio::process::Command::new("cargo");
    cmd.arg("build").arg("--bins").arg("--release").current_dir(&repo_path);

    if let Some(jobs) = args.jobs {
        cmd.arg("--jobs").arg(jobs.to_string());
    }

    let status = cmd.status().await.wrap_err("failed to run cargo build")?;
    if !status.success() {
        bail!("cargo build failed");
    }

    let version_dir = config.version_dir(&version);
    fs::create_dir_all(&version_dir)?;

    let bins = config.bins(args.network);
    for bin in bins {
        let src = repo_path.join("target/release").join(bin_name(bin));
        if src.exists() {
            fs::rename(&src, version_dir.join(bin_name(bin)))?;
        }
    }

    use_version(config, &version)?;
    say("done");

    Ok(())
}

async fn fetch_and_verify_attestation(
    config: &Config,
    downloader: &Downloader,
    release_url: &str,
    version: &str,
    target: &Target,
    bins: &[&str],
) -> Result<Option<HashMap<String, String>>> {
    say(&format!("checking if {} for {version} version are already installed", bins.join(", ")));

    let attestation_url = format!(
        "{release_url}foundry_{version}_{platform}_{arch}.attestation.txt",
        platform = target.platform.as_str(),
        arch = target.arch.as_str()
    );

    let attestation_link = match downloader.download_to_string(&attestation_url).await {
        Ok(content) => {
            let link = content.lines().next().unwrap_or("").trim().to_string();
            if link.is_empty() || link.contains("Not Found") {
                say("no attestation found for this release, skipping SHA verification");
                return Ok(None);
            }
            link
        }
        Err(_) => {
            say("no attestation found for this release, skipping SHA verification");
            return Ok(None);
        }
    };

    say(&format!(
        "found attestation for {version} version, downloading attestation artifact, checking..."
    ));

    let artifact_url = format!("{attestation_link}/download");
    let artifact_json = downloader.download_to_string(&artifact_url).await?;

    let hashes = parse_attestation_payload(&artifact_json)?;

    let tag = version_to_tag(version);
    let version_dir = config.version_dir(&tag);

    if version_dir.exists() {
        let mut all_match = true;
        for bin in bins {
            let bin_name = bin_name(bin);
            let expected = hashes.get(*bin).or_else(|| hashes.get(&bin_name));
            let path = version_dir.join(&bin_name);

            match expected {
                Some(expected_hash) if path.exists() => {
                    let actual = compute_sha256(&path)?;
                    if actual != *expected_hash {
                        all_match = false;
                        break;
                    }
                }
                _ => {
                    all_match = false;
                    break;
                }
            }
        }

        if all_match {
            say(&format!("version {tag} already installed and verified, activating..."));
            use_version(config, &tag)?;
            say("done!");
            std::process::exit(0);
        }
    }

    say("binaries not found or do not match expected hashes, downloading new binaries");
    Ok(Some(hashes))
}

fn parse_attestation_payload(json: &str) -> Result<HashMap<String, String>> {
    let parsed: serde_json::Value = serde_json::from_str(json)?;
    let payload_b64 = parsed["dsseEnvelope"]["payload"]
        .as_str()
        .ok_or_else(|| eyre::eyre!("missing payload in attestation"))?;

    let payload_bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload_b64)?;
    let payload_json: serde_json::Value = serde_json::from_slice(&payload_bytes)?;

    let mut hashes = HashMap::new();

    if let Some(subject) = payload_json["subject"].as_array() {
        for entry in subject {
            if let (Some(name), Some(digest)) =
                (entry["name"].as_str(), entry["digest"]["sha256"].as_str())
            {
                hashes.insert(name.to_string(), digest.to_string());
            }
        }
    }

    Ok(hashes)
}

async fn download_and_extract(
    config: &Config,
    downloader: &Downloader,
    release_url: &str,
    version: &str,
    tag: &str,
    target: &Target,
) -> Result<()> {
    let archive_name = format!(
        "foundry_{version}_{platform}_{arch}.{ext}",
        platform = target.platform.as_str(),
        arch = target.arch.as_str(),
        ext = target.platform.archive_ext()
    );

    let archive_url = format!("{release_url}{archive_name}");
    say(&format!("downloading {archive_name}"));

    let temp_dir = tempfile::tempdir()?;
    let archive_path = temp_dir.path().join(&archive_name);

    downloader.download_to_file(&archive_url, &archive_path).await?;

    let version_dir = config.version_dir(tag);
    fs::create_dir_all(&version_dir)?;

    if target.platform == Platform::Win32 {
        extract_zip(&archive_path, &version_dir)?;
    } else {
        extract_tar_gz(&archive_path, &version_dir)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in fs::read_dir(&version_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
            }
        }
    }

    Ok(())
}

async fn download_and_extract_tempo(
    config: &Config,
    downloader: &Downloader,
    release_url: &str,
    target: &Target,
    tag: &str,
) -> Result<()> {
    let archive_name = format!(
        "foundry_nightly_{platform}_{arch}.{ext}",
        platform = target.platform.as_str(),
        arch = target.arch.as_str(),
        ext = target.platform.archive_ext()
    );

    let archive_url = format!("{release_url}{archive_name}");
    say(&format!("downloading {archive_name}"));

    let temp_dir = tempfile::tempdir()?;
    let archive_path = temp_dir.path().join(&archive_name);

    downloader.download_to_file(&archive_url, &archive_path).await?;

    let version_dir = config.version_dir(tag);
    fs::create_dir_all(&version_dir)?;

    if target.platform == Platform::Win32 {
        extract_zip(&archive_path, &version_dir)?;
    } else {
        extract_tar_gz(&archive_path, &version_dir)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in fs::read_dir(&version_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
            }
        }
    }

    Ok(())
}

fn verify_installed_binaries(
    config: &Config,
    tag: &str,
    bins: &[&str],
    hashes: &HashMap<String, String>,
) -> Result<()> {
    say("verifying downloaded binaries against the attestation file");

    let version_dir = config.version_dir(tag);
    let mut failed = false;

    for bin in bins {
        let bin_name = bin_name(bin);
        let expected = hashes.get(*bin).or_else(|| hashes.get(&bin_name));
        let path = version_dir.join(&bin_name);

        match expected {
            None => {
                say(&format!("no expected hash for {bin}"));
                failed = true;
            }
            Some(expected_hash) => {
                if !path.exists() {
                    say(&format!("binary {bin} not found at {}", path.display()));
                    failed = true;
                    continue;
                }

                let actual = compute_sha256(&path)?;
                if actual != *expected_hash {
                    say(&format!("{bin} hash verification failed:"));
                    say(&format!("  expected: {expected_hash}"));
                    say(&format!("  actual:   {actual}"));
                    failed = true;
                } else {
                    say(&format!("{bin} verified ✓"));
                }
            }
        }
    }

    if failed {
        bail!("one or more binaries failed post-installation verification");
    }

    Ok(())
}

async fn download_manpages(
    config: &Config,
    downloader: &Downloader,
    release_url: &str,
    version: &str,
) {
    let man_url = format!("{release_url}foundry_man_{version}.tar.gz");
    say("downloading manpages");

    let temp_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => {
            warn("skipping manpage download: failed to create temp directory");
            return;
        }
    };
    let archive_path = temp_dir.path().join("foundry_man.tar.gz");

    if downloader.download_to_file(&man_url, &archive_path).await.is_err() {
        warn("skipping manpage download: unavailable or invalid archive");
        return;
    }

    if let Err(e) = extract_tar_gz(&archive_path, &config.man_dir) {
        warn(&format!("skipping manpage download: {e}"));
    }
}

pub(crate) fn list(config: &Config) -> Result<()> {
    let bins = config.bins(None);

    if config.versions_dir.exists() {
        for entry in fs::read_dir(&config.versions_dir)? {
            let entry = entry?;
            let version_name = entry.file_name();
            let version_name = version_name.to_string_lossy();

            say(&version_name);

            for bin in bins {
                let bin_path = entry.path().join(bin_name(bin));
                if bin_path.exists() {
                    match get_bin_version(&bin_path) {
                        Ok(v) => say(&format!("- {v}")),
                        Err(_) => say(&format!("- {bin} (unknown version)")),
                    }
                }
            }
            eprintln!();
        }
    } else {
        for bin in bins {
            let bin_path = config.bin_path(bin);
            if bin_path.exists() {
                match get_bin_version(&bin_path) {
                    Ok(v) => say(&format!("- {v}")),
                    Err(_) => say(&format!("- {bin} (unknown version)")),
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn use_version(config: &Config, version: &str) -> Result<()> {
    let version_dir = config.version_dir(version);

    if !version_dir.exists() {
        bail!("version {version} not installed");
    }

    let bins = config.bins(None);

    for bin in bins {
        let bin_name = bin_name(bin);
        let src = version_dir.join(&bin_name);
        let dest = config.bin_path(bin);

        if !src.exists() {
            continue;
        }

        if dest.exists() {
            fs::remove_file(&dest)?;
        }

        fs::copy(&src, &dest)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
        }

        match get_bin_version(&dest) {
            Ok(v) => say(&format!("use - {v}")),
            Err(_) => say(&format!("use - {bin}")),
        }

        if let Ok(which_path) = which::which(bin) {
            if which_path != dest {
                warn("");
                eprintln!(
                    r#"There are multiple binaries with the name '{bin}' present in your 'PATH'.
This may be the result of installing '{bin}' using another method,
like Cargo or other package managers.
You may need to run 'rm {which_path}' or move '{bin_dir}'
in your 'PATH' to allow the newly installed version to take precedence!
"#,
                    which_path = which_path.display(),
                    bin_dir = config.bin_dir.display()
                );
            }
        }
    }

    Ok(())
}

fn normalize_version(version: &str) -> String {
    if version.starts_with("nightly") {
        "nightly".to_string()
    } else if version.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        format!("v{version}")
    } else {
        version.to_string()
    }
}

fn version_to_tag(version: &str) -> String {
    version.to_string()
}

fn bin_name(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

fn get_bin_version(path: &Path) -> Result<String> {
    let output = std::process::Command::new(path).arg("-V").output()?;
    let version = String::from_utf8_lossy(&output.stdout);
    Ok(version.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attestation_de() {
        let s = r#"{
          "mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json",
          "verificationMaterial": {
            "tlogEntries": [
              {
                "logIndex": "726844033",
                "logId": {
                  "keyId": "wNI9atQGlz+VWfO6LRygH4QUfY/8W4RFwiT5i5WRgB0="
                },
                "kindVersion": {
                  "kind": "dsse",
                  "version": "0.0.1"
                },
                "integratedTime": "1764149163",
                "inclusionPromise": {
                  "signedEntryTimestamp": "MEQCICQ4vKUag1Ie7qUZ3tixCbhHvpL9nCk6AxsoNH8foRlIAiB3ZuvlVkJNyk8GWs8DriDd74ywGXS/DNWFCGruKfImzA=="
                },
                "inclusionProof": {
                  "logIndex": "604939771",
                  "rootHash": "pMLuZ9LswMdPA8hK2gigUVdmpRDdhVGTdXXHHuK9i5A=",
                  "treeSize": "604939772",
                  "hashes": [
                    "ZOpcN0IkZasxt47RXbTVd4cLMzb4uDya4+HWroLY/9Q=",
                    "0yzLD+HRXojb8IZbbYK6L6HRQuoGkw0lNLSvDVI2K6w=",
                    "athwre7ChD6XJdeoGK+kIUlkaoPSl0GsVJI2aXuaXCs=",
                    "yQPDaEVBYDwdmek4efsisyqxB5ur6/2dw7SdL7KO2gk=",
                    "L5Z4Fzb+NFymGxjzj1m43TJNKeUxa6Br94Yc/JKGi8c=",
                    "zPAiix3Iu1JtTq6D7Lnf0Asmw5isvQSg5IvtTtwHo8Y=",
                    "c7mZfLxzSRxVx8bnVoI8t8eIVIATKhaX1urSlh8EQVQ=",
                    "XluODcZs3Wy4m2OtgK/PNM5jCsh8gKRIjw1l0ZFiHHg=",
                    "ET1+ajsPyYg1dltnPNH3Qq/oPy+jaQD7anORn7f00Bg=",
                    "Wm/MvwCBf55Q7PWrwIqdEXe2b0bZdsOg6Jouo6J+Trc=",
                    "fFWBsilqrAx02jL52CmpU+qvaaIjynrm5nIT4IAURc8=",
                    "WoVJpFMwUpz1XAIY6HJIUS/6kNtjomdGoooeMqPxhoQ=",
                    "o6nbDxwthgai9Fxn+LQ9YOau/WdIt9iePVI9bgKrtVc=",
                    "IQFnPqg26SCaobVnQILSdO05Znh97ys4y0IThJXH0Kc=",
                    "ZmUkYkHBy1B723JrEgiKvepTdHYrP6y2a4oODYvi5VY=",
                    "T4DqWD42hAtN+vX8jKCWqoC4meE4JekI9LxYGCcPy1M="
                  ],
                  "checkpoint": {
                    "envelope": "rekor.sigstore.dev - 1193050959916656506\n604939772\npMLuZ9LswMdPA8hK2gigUVdmpRDdhVGTdXXHHuK9i5A=\n\n— rekor.sigstore.dev wNI9ajBGAiEA0edmUQ86q0DrZPl295Agpgnf2LBXL/fUYQ6LFu72kuICIQDCS0hMHJjnxgj1vmV4mbBNzuGhGSvS8FiCQSTcnWoGzQ==\n"
                  }
                },
                "canonicalizedBody": "eyJhcGlWZXJzaW9uIjoiMC4wLjEiLCJraW5kIjoiZHNzZSIsInNwZWMiOnsiZW52ZWxvcGVIYXNoIjp7ImFsZ29yaXRobSI6InNoYTI1NiIsInZhbHVlIjoiOGMzZTBiMjI4MzlmYzc3OTE4NzYzYjlkMzdkZTc4MzYyMDk5YTdkNGRlZjcxNDU4Nzg5ZjZiZGE3M2MxYzUyMiJ9LCJwYXlsb2FkSGFzaCI6eyJhbGdvcml0aG0iOiJzaGEyNTYiLCJ2YWx1ZSI6IjNjNGFhMmFiNDg4OTYzMzg2ZjljYWExOGJkNWNiOTI2YWM3OTc3MDJmZThhZTkzOTAwNjc5ODE1ZWZiYTFkY2IifSwic2lnbmF0dXJlcyI6W3sic2lnbmF0dXJlIjoiTUVZQ0lRQ1FmOTd5SXpaMkMydFg4clJ5S05LRlFFZGxIbDJhbmlFR3c2eFY2MTJNT1FJaEFJZzQ4UkRuMHR0Q3k5WEpkblY0M2k5YUZjQzMrTVFuWStBbmxTREx4dE1MIiwidmVyaWZpZXIiOiJMUzB0TFMxQ1JVZEpUaUJEUlZKVVNVWkpRMEZVUlMwdExTMHRDazFKU1VkeWVrTkRRbXBUWjBGM1NVSkJaMGxWWVZoQlZtbHdUbVJ6YWs5TGRVUmFTMlpuVkVKTlJHVlBLMjluZDBObldVbExiMXBKZW1vd1JVRjNUWGNLVG5wRlZrMUNUVWRCTVZWRlEyaE5UV015Ykc1ak0xSjJZMjFWZFZwSFZqSk5ValIzU0VGWlJGWlJVVVJGZUZaNllWZGtlbVJIT1hsYVV6RndZbTVTYkFwamJURnNXa2RzYUdSSFZYZElhR05PVFdwVmVFMVVTVEpOUkd0NVRtcEJlbGRvWTA1TmFsVjRUVlJKTWsxRWEzcE9ha0Y2VjJwQlFVMUdhM2RGZDFsSUNrdHZXa2w2YWpCRFFWRlpTVXR2V2tsNmFqQkVRVkZqUkZGblFVVXlNRlZ1UzBWT2NVc3JSWFJQWms1WFl6bDRLMGRZUldwaWJrMVFaV3h2V1N0Sk5rc0tPSFk0VFhOT1NscGpSMkZrWkZaRWVFdE1OV05vYnpJMVpXVTJLMmhMVTNrdk1YcHpTSE5qZGtOWUsyMUVRVFk1WTJGUFEwSldUWGRuWjFaUVRVRTBSd3BCTVZWa1JIZEZRaTkzVVVWQmQwbElaMFJCVkVKblRsWklVMVZGUkVSQlMwSm5aM0pDWjBWR1FsRmpSRUY2UVdSQ1owNVdTRkUwUlVablVWVlRjekprQ21wVWEzSlNLeXM1TlU5WVlVc3dNRUpSZG5GMUwwWjNkMGgzV1VSV1VqQnFRa0puZDBadlFWVXpPVkJ3ZWpGWmEwVmFZalZ4VG1wd1MwWlhhWGhwTkZrS1drUTRkMWxuV1VSV1VqQlNRVkZJTDBKR1ozZFdiMXBWWVVoU01HTklUVFpNZVRsdVlWaFNiMlJYU1hWWk1qbDBUREphZG1SWE5XdGpibXQwWTI1TmRncGFiVGt4WW0xU2VXVlRPSFZhTW13d1lVaFdhVXd6WkhaamJYUnRZa2M1TTJONU9YbGFWM2hzV1ZoT2JFeHViSFJpUlVKNVdsZGFla3d6VW1oYU0wMTJDbU16VW1oWmJYaHNUVVJyUjBOcGMwZEJVVkZDWnpjNGQwRlJSVVZMTW1nd1pFaENlazlwT0haa1J6bHlXbGMwZFZsWFRqQmhWemwxWTNrMWJtRllVbThLWkZkS01XTXlWbmxaTWpsMVpFZFdkV1JETldwaU1qQjNSV2RaUzB0M1dVSkNRVWRFZG5wQlFrRm5VVVZqU0ZaNllVUkJNa0puYjNKQ1owVkZRVmxQTHdwTlFVVkVRa05uZUZsNlZUTlBSRlV3VGtSWmVVMXFaelZaYWtwc1RucEdiRnBVWXpKT1ZGSnFXa1JaTWs1cVdYbE5WR1JzV2tSbk1scHRXbXROUWxWSENrTnBjMGRCVVZGQ1p6YzRkMEZSVVVWQ00wcHNZa2RXYUdNeVZYZEpRVmxMUzNkWlFrSkJSMFIyZWtGQ1FsRlJVMXB0T1RGaWJWSjVaVk14ZVdONU9XMEtZak5XZFZwSVNqVk5RalJIUTJselIwRlJVVUpuTnpoM1FWRlpSVVZJU214YWJrMTJaRWRHYm1ONU9YcGtSMFpwWWtkVmQwOTNXVXRMZDFsQ1FrRkhSQXAyZWtGQ1EwRlJkRVJEZEc5a1NGSjNZM3B2ZGt3elVuWmhNbFoxVEcxR2FtUkhiSFppYmsxMVdqSnNNR0ZJVm1sa1dFNXNZMjFPZG1KdVVteGlibEYxQ2xreU9YUk5SMUZIUTJselIwRlJVVUpuTnpoM1FWRnJSVlpuZUZWaFNGSXdZMGhOTmt4NU9XNWhXRkp2WkZkSmRWa3lPWFJNTWxwMlpGYzFhMk51YTNRS1kyNU5kbHB0T1RGaWJWSjVaVk00ZFZveWJEQmhTRlpwVEROa2RtTnRkRzFpUnprelkzazVlVnBYZUd4WldFNXNURzVzZEdKRlFubGFWMXA2VEROU2FBcGFNMDEyWXpOU2FGbHRlR3hOUkdkSFEybHpSMEZSVVVKbk56aDNRVkZ2UlV0bmQyOU5WMDB4VG5wbk1VNUVVVEpOYWtrMFQxZEplVnBVWTNoYVYxVXpDazVxVlRCWk1sRXlUbXBaTWsxcVJUTmFWMUUwVG0xYWJWcEVRV0pDWjI5eVFtZEZSVUZaVHk5TlFVVk1Ra0V3VFVNelRteGlSMWwwWVVjNWVtUkhWbXNLVFVSVlIwTnBjMGRCVVZGQ1p6YzRkMEZSZDBWS2QzZHNZVWhTTUdOSVRUWk1lVGx1WVZoU2IyUlhTWFZaTWpsMFRESmFkbVJYTld0amJtdDBZMjVOZGdwYWJUa3hZbTFTZVdWVVFUUkNaMjl5UW1kRlJVRlpUeTlOUVVWT1FrTnZUVXRFUm1wT1ZHTTBUbFJSTUU1cVNYbFBSR3hwVFcxVk0wMVhWbXhPZWxreENrNUhUbXRPYWxreVRtcEplRTR5Vm10UFJGcHRXbTFSZDBsQldVdExkMWxDUWtGSFJIWjZRVUpFWjFGVFJFSkNlVnBYV25wTU0xSm9Xak5OZG1NelVtZ0tXVzE0YkUxQ2EwZERhWE5IUVZGUlFtYzNPSGRCVVRoRlEzZDNTazVFUVRCTmVrbDNUVVJWZWsxRE1FZERhWE5IUVZGUlFtYzNPSGRCVWtGRlNIZDNaQXBoU0ZJd1kwaE5Oa3g1T1c1aFdGSnZaRmRKZFZreU9YUk1NbHAyWkZjMWEyTnVhM1JqYmsxM1IwRlpTMHQzV1VKQ1FVZEVkbnBCUWtWUlVVdEVRV2MxQ2s5VVp6Vk5hbEUxVGtSQ2EwSm5iM0pDWjBWRlFWbFBMMDFCUlZOQ1JsbE5Wa2RvTUdSSVFucFBhVGgyV2pKc01HRklWbWxNYlU1MllsTTViV0l6Vm5VS1draEtOVXhZU25wTU1scDJaRmMxYTJOdWEzWk1iV1J3WkVkb01WbHBPVE5pTTBweVdtMTRkbVF6VFhaamJWWnpXbGRHZWxwVE5UVmlWM2hCWTIxV2JRcGplVGt3V1Zka2Vrd3pUakJaVjBweldsUkJORUpuYjNKQ1owVkZRVmxQTDAxQlJWUkNRMjlOUzBSR2FrNVVZelJPVkZFd1RtcEplVTlFYkdsTmJWVXpDazFYVm14T2Vsa3hUa2RPYTA1cVdUSk9ha2w0VGpKV2EwOUVXbTFhYlZGM1JrRlpTMHQzV1VKQ1FVZEVkbnBCUWtaQlVVZEVRVkozWkZoT2IwMUdhMGNLUTJselIwRlJVVUpuTnpoM1FWSlZSVk4zZUVwaFNGSXdZMGhOTmt4NU9XNWhXRkp2WkZkSmRWa3lPWFJNTWxwMlpGYzFhMk51YTNSamJrMTJXbTA1TVFwaWJWSjVaVk01YUZrelVuQmlNalY2VEROS01XSnVUWFpOVkdzeVQxUm5NRTVFVFRST2FsRjJXVmhTTUZwWE1YZGtTRTEyVFZSQlYwSm5iM0pDWjBWRkNrRlpUeTlOUVVWWFFrRm5UVUp1UWpGWmJYaHdXWHBEUW1sUldVdExkMWxDUWtGSVYyVlJTVVZCWjFJM1FraHJRV1IzUWpGQlRqQTVUVWR5UjNoNFJYa0tXWGhyWlVoS2JHNU9kMHRwVTJ3Mk5ETnFlWFF2TkdWTFkyOUJka3RsTms5QlFVRkNiWEk1Tnpsb05FRkJRVkZFUVVWWmQxSkJTV2RGY0dOQ00yZ3ZVUXBuT0UwNFdrdEtLelUyWjNweE1HeG5RWHBsUlhvNGNYVmFkR0ZUVDJacVZtdGFaME5KUVhGMlJDOTNVVzFaZG5sUmJtdG9ZVTVzTmtkeldFZGhRVUZaQ21SNGJtbEtaMGhJUjFaQ01qQkpSM2xOUVc5SFEwTnhSMU5OTkRsQ1FVMUVRVEpyUVUxSFdVTk5VVVF4VkRkQmVqQmhiblJVTlVOdmRVOTZNM2hpWXpZS1VpdHJiRWQ1V0hKbFRHZ3pPRkU0TWt4bll6Uk5TVGR4YTNCWldEWmhUM1JLVm1ST2NtWmlkWGgxVVVOTlVVTlJNVVZVYjIxVFZtWkljSGhMUnpsdE5BcHZRVEpMVjBSaWJVUk5ZMHRoUmpGdVJXWjBTRUppYldzeVNFWkVXVVZKVXpjemFESXJUMWw2TjNaNVZXbGlhejBLTFMwdExTMUZUa1FnUTBWU1ZFbEdTVU5CVkVVdExTMHRMUW89In1dfX0="
              }
            ],
            "timestampVerificationData": {},
            "certificate": {
              "rawBytes": "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"
            }
          },
          "dsseEnvelope": {
            "payload": "eyJfdHlwZSI6Imh0dHBzOi8vaW4tdG90by5pby9TdGF0ZW1lbnQvdjEiLCJzdWJqZWN0IjpbeyJuYW1lIjoiYW52aWwiLCJkaWdlc3QiOnsic2hhMjU2IjoiZGRkMGE1OTc0NDUxNjQyNDA0YjZhMzQ4NWY5NWViMzVjYTVmYjU4ZTRhODBhYzIyMDA0Y2EzZTMyMjlhYWJjMCJ9fSx7Im5hbWUiOiJjYXN0IiwiZGlnZXN0Ijp7InNoYTI1NiI6ImQ4Zjg3NzNhNWI0MWFjODIzMzZmMzJiZGI1MjkzODBkY2NlNDJkNDQxYTM3NzBiYWUxMDZlNzlkZGFhMjE4ZjUifX0seyJuYW1lIjoiY2hpc2VsIiwiZGlnZXN0Ijp7InNoYTI1NiI6IjVhODRjNWMwNTRiOWM4ZjdjMWRhYjVjN2Y3MDE0Y2JkOGUxOGRlNDYyZmYyNGY0ODhiMmI3ZDc5YjRmNGJmY2QifX0seyJuYW1lIjoiZm9yZ2UiLCJkaWdlc3QiOnsic2hhMjU2IjoiNjhkOTUzN2MzMjkwN2Y0M2EwYmIyYWVhM2UyYmMxMmE3MzI2YmZjOTA2ZTI2OTA0ZGZmYWQyZDM1NWY3NDYxZiJ9fV0sInByZWRpY2F0ZVR5cGUiOiJodHRwczovL3Nsc2EuZGV2L3Byb3ZlbmFuY2UvdjEiLCJwcmVkaWNhdGUiOnsiYnVpbGREZWZpbml0aW9uIjp7ImJ1aWxkVHlwZSI6Imh0dHBzOi8vYWN0aW9ucy5naXRodWIuaW8vYnVpbGR0eXBlcy93b3JrZmxvdy92MSIsImV4dGVybmFsUGFyYW1ldGVycyI6eyJ3b3JrZmxvdyI6eyJyZWYiOiJyZWZzL3RhZ3Mvc3RhYmxlIiwicmVwb3NpdG9yeSI6Imh0dHBzOi8vZ2l0aHViLmNvbS9mb3VuZHJ5LXJzL2ZvdW5kcnkiLCJwYXRoIjoiLmdpdGh1Yi93b3JrZmxvd3MvcmVsZWFzZS55bWwifX0sImludGVybmFsUGFyYW1ldGVycyI6eyJnaXRodWIiOnsiZXZlbnRfbmFtZSI6InB1c2giLCJyZXBvc2l0b3J5X2lkIjoiNDA0MzIwMDUzIiwicmVwb3NpdG9yeV9vd25lcl9pZCI6Ijk5ODkyNDk0IiwicnVubmVyX2Vudmlyb25tZW50Ijoic2VsZi1ob3N0ZWQifX0sInJlc29sdmVkRGVwZW5kZW5jaWVzIjpbeyJ1cmkiOiJnaXQraHR0cHM6Ly9naXRodWIuY29tL2ZvdW5kcnktcnMvZm91bmRyeUByZWZzL3RhZ3Mvc3RhYmxlIiwiZGlnZXN0Ijp7ImdpdENvbW1pdCI6IjFjNTc4NTQ0NjIyODliMmU3MWVlNzY1NGNkNjY2NjIxN2VkODZmZmQifX1dfSwicnVuRGV0YWlscyI6eyJidWlsZGVyIjp7ImlkIjoiaHR0cHM6Ly9naXRodWIuY29tL2ZvdW5kcnktcnMvZm91bmRyeS8uZ2l0aHViL3dvcmtmbG93cy9yZWxlYXNlLnltbEByZWZzL3RhZ3Mvc3RhYmxlIn0sIm1ldGFkYXRhIjp7Imludm9jYXRpb25JZCI6Imh0dHBzOi8vZ2l0aHViLmNvbS9mb3VuZHJ5LXJzL2ZvdW5kcnkvYWN0aW9ucy9ydW5zLzE5Njk4NDQzODY0L2F0dGVtcHRzLzEifX19fQ==",
            "payloadType": "application/vnd.in-toto+json",
            "signatures": [
              {
                "sig": "MEYCIQCQf97yIzZ2C2tX8rRyKNKFQEdlHl2aniEGw6xV612MOQIhAIg48RDn0ttCy9XJdnV43i9aFcC3+MQnY+AnlSDLxtML"
              }
            ]
          }
        }"#;

        let hashes = parse_attestation_payload(s).unwrap();
        assert!(!hashes.is_empty());
    }
}
