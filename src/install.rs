use crate::{
    cli::{InstallArgs, Network},
    config::Config,
    download::{Downloader, compute_sha256, extract_tar_gz, extract_zip},
    platform::{Platform, Target},
    say, warn,
};
use eyre::{Result, WrapErr, bail};
use std::{collections::HashMap, path::Path};

pub(crate) async fn run(config: &Config, args: InstallArgs) -> Result<()> {
    config.ensure_dirs()?;

    if args.pr.is_some() && args.branch.is_some() {
        bail!("can't use --pr and --branch at the same time");
    }

    if let Some(ref local_path) = args.path {
        return install_from_local(config, local_path, &args).await;
    }

    let network = args.network;
    let repo = args.repo.as_deref().unwrap_or_else(|| config.repo(network));

    let should_build = args.branch.is_some() || args.pr.is_some() || args.commit.is_some();
    let is_foundry_repo = repo == crate::config::FOUNDRY_REPO;
    let is_tempo = network == Some(Network::Tempo);

    if is_foundry_repo && !should_build {
        install_prebuilt(config, &args).await
    } else if is_tempo && !should_build {
        install_tempo_prebuilt(config, &args).await
    } else {
        install_from_source(config, repo, &args).await
    }
}

async fn install_prebuilt(config: &Config, args: &InstallArgs) -> Result<()> {
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

async fn install_tempo_prebuilt(config: &Config, args: &InstallArgs) -> Result<()> {
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

async fn install_from_local(config: &Config, local_path: &Path, args: &InstallArgs) -> Result<()> {
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
            std::fs::remove_file(&dest)?;
        }

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&src, &dest)
                .wrap_err_with(|| format!("failed to symlink {}", bin))?;
        }
        #[cfg(windows)]
        {
            std::fs::copy(&src, &dest).wrap_err_with(|| format!("failed to copy {}", bin))?;
        }
    }

    say("done");
    Ok(())
}

async fn install_from_source(config: &Config, repo: &str, args: &InstallArgs) -> Result<()> {
    let branch = if let Some(pr) = args.pr {
        format!("refs/pull/{pr}/head")
    } else {
        args.branch.clone().unwrap_or_else(|| "master".to_string())
    };

    let repo_path = config.repo_dir(repo);
    let author = repo.split('/').next().unwrap_or(repo);

    if !repo_path.exists() {
        let author_dir = config.foundry_dir.join(author);
        std::fs::create_dir_all(&author_dir)?;

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
    std::fs::create_dir_all(&version_dir)?;

    let bins = config.bins(args.network);
    for bin in bins {
        let src = repo_path.join("target/release").join(bin_name(bin));
        if src.exists() {
            std::fs::rename(&src, version_dir.join(bin_name(bin)))?;
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
    let payload_b64 =
        parsed["payload"].as_str().ok_or_else(|| eyre::eyre!("missing payload in attestation"))?;

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
    std::fs::create_dir_all(&version_dir)?;

    if target.platform == Platform::Win32 {
        extract_zip(&archive_path, &version_dir)?;
    } else {
        extract_tar_gz(&archive_path, &version_dir)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in std::fs::read_dir(&version_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
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
    std::fs::create_dir_all(&version_dir)?;

    if target.platform == Platform::Win32 {
        extract_zip(&archive_path, &version_dir)?;
    } else {
        extract_tar_gz(&archive_path, &version_dir)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in std::fs::read_dir(&version_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
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
        for entry in std::fs::read_dir(&config.versions_dir)? {
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
            std::fs::remove_file(&dest)?;
        }

        std::fs::copy(&src, &dest).wrap_err_with(|| format!("failed to copy {bin}"))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
        }

        match get_bin_version(&dest) {
            Ok(v) => say(&format!("use - {v}")),
            Err(_) => say(&format!("use - {bin}")),
        }

        if let Some(which_path) = which(bin) {
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

fn which(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let path = dir.join(bin_name(name));
            if path.is_file() { Some(path) } else { None }
        })
    })
}
