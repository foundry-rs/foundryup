use crate::{
    cli::Cli,
    config::Config,
    download::{Downloader, compute_sha256, extract_tar_gz, extract_zip},
    platform::{Platform, Target},
    say, tell, warn,
};
use eyre::{Result, WrapErr, bail};
use fs_err as fs;
use std::{collections::HashMap, path::Path};

pub(crate) async fn run(config: &Config, args: &Cli) -> Result<()> {
    config.ensure_dirs()?;

    if let Some(ref local_path) = args.path {
        return install_from_local(config, local_path, args).await;
    }

    let repo = args.repo.as_deref().unwrap_or(config.network.repo);

    let should_build = args.branch.is_some() || args.pr.is_some() || args.commit.is_some();
    let is_default_repo = repo == config.network.repo;

    if is_default_repo && !should_build {
        install_prebuilt(config, args).await
    } else {
        install_from_source(config, repo, args).await
    }
}

async fn install_prebuilt(config: &Config, args: &Cli) -> Result<()> {
    let repo = config.network.repo;

    let downloader = Downloader::new()?;

    let (version, tag) = resolve_version_and_tag(
        &downloader,
        repo,
        args.version.as_deref().unwrap_or(config.network.default_version),
    )
    .await?;

    say!("installing {} (version {version}, tag {tag})", config.network.display_name);

    let target = Target::detect(args.platform.as_deref(), args.arch.as_deref())?;

    let release_url =
        format!("https://github.com/{}/releases/download/{tag}/", config.network.repo);

    let check = if config.network.has_attestation && !args.force {
        fetch_and_verify_attestation(
            config,
            repo,
            &downloader,
            &release_url,
            &version,
            &tag,
            &target,
        )
        .await?
    } else {
        if args.force {
            say!("skipped SHA verification due to --force flag");
        }
        PrebuiltCheck::Download(None)
    };

    // A verified cache hit is already activated; return so `main` can still
    // print the foundryup self-update check instead of exiting the process here.
    let hashes = match check {
        PrebuiltCheck::AlreadyActivated => return Ok(()),
        PrebuiltCheck::Download(hashes) => hashes,
    };

    download_and_extract(config, repo, &downloader, &release_url, &version, &tag, &target).await?;

    if let Some(ref hashes) = hashes {
        verify_installed_binaries(config, repo, &tag, hashes)?;
    }

    download_manpages(config, &downloader, &release_url, &version).await;

    use_version(config, repo, &tag)?;
    say!("done!");

    Ok(())
}

async fn install_from_local(config: &Config, local_path: &Path, args: &Cli) -> Result<()> {
    need_cmd("cargo")?;

    if args.repo.is_some() || args.branch.is_some() || args.version.is_some() {
        warn!("--branch, --install, --use, and --repo arguments are ignored during local install");
    }

    // Resolve to an absolute path so the symlinks below point at the build
    // artifacts regardless of the current working directory.
    let local_path = fs::canonicalize(local_path)
        .wrap_err_with(|| format!("local repository not found: {}", local_path.display()))?;
    let local_path = local_path.as_path();

    say!("installing from {}", local_path.display());

    let mut cmd = tokio::process::Command::new("cargo");
    cmd.arg("build")
        .arg("--bins")
        .arg("--profile")
        .arg(&args.cargo_profile)
        .current_dir(local_path);
    cmd.env("RUSTFLAGS", rustflags());

    if let Some(jobs) = args.jobs {
        cmd.arg("--jobs").arg(jobs.to_string());
    }
    if let Some(ref features) = args.cargo_features {
        cmd.arg("--features").arg(features);
    }

    let status = cmd.status().await.wrap_err("failed to run cargo build")?;
    if !status.success() {
        bail!("cargo build failed");
    }

    let target_dir = profile_target_dir(&args.cargo_profile);
    for bin in config.network.bins {
        let src = local_path.join("target").join(target_dir).join(bin_name(bin));
        let dest = config.bin_path(bin);

        clear_active_bin(config, bin)?;

        #[cfg(unix)]
        std::os::unix::fs::symlink(&src, &dest)?;
        #[cfg(windows)]
        fs::copy(&src, &dest)?;
    }

    say!("done");
    Ok(())
}

fn profile_target_dir(profile: &str) -> &str {
    if profile == "dev" { "debug" } else { profile }
}

async fn install_from_source(config: &Config, repo: &str, args: &Cli) -> Result<()> {
    need_cmd("git")?;
    need_cmd("cargo")?;

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

        say!("cloning {repo}...");
        let status = tokio::process::Command::new("git")
            .args(["clone", &format!("https://github.com/{repo}")])
            .current_dir(&author_dir)
            .status()
            .await?;
        if !status.success() {
            bail!("git clone failed");
        }
    }

    say!("fetching {branch}...");
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

    say!("installing version {version}");

    let mut cmd = tokio::process::Command::new("cargo");
    cmd.arg("build")
        .arg("--bins")
        .arg("--profile")
        .arg(&args.cargo_profile)
        .current_dir(&repo_path);
    cmd.env("RUSTFLAGS", rustflags());

    if let Some(jobs) = args.jobs {
        cmd.arg("--jobs").arg(jobs.to_string());
    }
    if let Some(ref features) = args.cargo_features {
        cmd.arg("--features").arg(features);
    }

    let status = cmd.status().await.wrap_err("failed to run cargo build")?;
    if !status.success() {
        bail!("cargo build failed");
    }

    let version_dir = config.version_dir(repo, &version);
    fs::create_dir_all(&version_dir)?;

    let target_dir = profile_target_dir(&args.cargo_profile);
    let build_dir = repo_path.join("target").join(target_dir);
    for bin in config.network.bins {
        // Move whichever artifact cargo produced (with or without `.exe`).
        for candidate in [bin.to_string(), format!("{bin}.exe")] {
            let src = build_dir.join(&candidate);
            if src.is_file() {
                // Remove any existing destination first; `fs::rename` fails on
                // Windows if the target already exists.
                let dest = version_dir.join(&candidate);
                remove_if_exists(&dest)?;
                fs::rename(&src, &dest)?;
            }
        }
    }

    use_version(config, repo, &version)?;

    generate_manpages_from_source(config).await?;

    say!("done");

    Ok(())
}

/// Generates man pages from the freshly built binaries using `help2man`,
/// writing each `<bin>.1` into the man directory. Skipped when `help2man` is
/// not installed.
async fn generate_manpages_from_source(config: &Config) -> Result<()> {
    if which::which("help2man").is_err() {
        return Ok(());
    }

    for bin in config.network.bins {
        let bin_path = config.bin_path(bin);
        let man_path = config.man_dir.join(format!("{bin}.1"));

        // Create/truncate the destination first, matching the shell's `> file`.
        let man_file = fs::File::create(&man_path)?;

        let status = tokio::process::Command::new("help2man")
            .arg("-N")
            .arg(&bin_path)
            .stdout(std::process::Stdio::from(man_file.into_parts().0))
            .status()
            .await
            .wrap_err_with(|| format!("failed to run help2man for {bin}"))?;

        if !status.success() {
            bail!("help2man failed for {bin}");
        }
    }

    Ok(())
}

/// Outcome of the pre-download attestation check for a prebuilt install.
enum PrebuiltCheck {
    /// The requested version was already installed and verified, and has been
    /// activated. The caller should finish without downloading anything.
    AlreadyActivated,
    /// The binaries must be downloaded. The optional hashes are used to verify
    /// them afterwards (`None` when the release has no attestation).
    Download(Option<HashMap<String, String>>),
}

async fn fetch_and_verify_attestation(
    config: &Config,
    repo: &str,
    downloader: &Downloader,
    release_url: &str,
    version: &str,
    tag: &str,
    target: &Target,
) -> Result<PrebuiltCheck> {
    let bins = config.network.bins;
    say!("checking if {} for {tag} version are already installed", bins.join(", "));

    let attestation_url = format!(
        "{release_url}foundry_{version}_{platform}_{arch}.attestation.txt",
        platform = target.platform.as_str(),
        arch = target.arch.as_str()
    );

    // A 404 (genuinely absent attestation) skips verification. Transport failures and other HTTP
    // errors are propagated so a network blip aborts the install rather than silently
    // downgrading to an unverified binary.
    let attestation_link = match downloader.download_to_string_optional(&attestation_url).await? {
        Some(content) => {
            let link = content.lines().next().unwrap_or("").trim().to_string();
            if link.is_empty() || link.contains("Not Found") {
                say!("no attestation found for this release, skipping SHA verification");
                return Ok(PrebuiltCheck::Download(None));
            }
            link
        }
        None => {
            say!("no attestation found for this release, skipping SHA verification");
            return Ok(PrebuiltCheck::Download(None));
        }
    };

    say!("found attestation for {tag} version, downloading attestation artifact, checking...");

    let artifact_url = format!("{attestation_link}/download");
    let artifact_json = downloader.download_to_string(&artifact_url).await?;

    let hashes = parse_attestation_payload(&artifact_json)?;

    let version_dir = config.version_dir(repo, tag);

    if version_dir.exists() {
        let mut all_match = true;
        for bin in bins {
            let bin_name = bin_name(bin);
            let expected = hashes.get(*bin).or_else(|| hashes.get(&bin_name));
            let path = version_dir.join(&bin_name);

            match expected {
                Some(expected_hash) if is_executable(&path) => {
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
            say!("version {tag} already installed and verified, activating...");
            use_version(config, repo, tag)?;
            say!("done!");
            return Ok(PrebuiltCheck::AlreadyActivated);
        }
    }

    say!("binaries not found or do not match expected hashes, downloading new binaries");
    Ok(PrebuiltCheck::Download(Some(hashes)))
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
    repo: &str,
    downloader: &Downloader,
    release_url: &str,
    version: &str,
    tag: &str,
    target: &Target,
) -> Result<()> {
    let archive_name = format!(
        "{prefix}_{version}_{platform}_{arch}.{ext}",
        prefix = config.network.archive_prefix,
        platform = target.platform.as_str(),
        arch = target.arch.as_str(),
        ext = target.platform.archive_ext()
    );

    let archive_url = format!("{release_url}{archive_name}");
    say!("downloading {archive_name}");

    let temp_dir = tempfile::tempdir()?;
    let archive_path = temp_dir.path().join(&archive_name);

    downloader.download_to_file(&archive_url, &archive_path).await?;

    let version_dir = config.version_dir(repo, tag);
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
    repo: &str,
    tag: &str,
    hashes: &HashMap<String, String>,
) -> Result<()> {
    say!("verifying downloaded binaries against the attestation file");

    let version_dir = config.version_dir(repo, tag);
    let mut failed = false;

    for bin in config.network.bins {
        let bin_name = bin_name(bin);
        let expected = hashes.get(*bin).or_else(|| hashes.get(&bin_name));
        let path = version_dir.join(&bin_name);

        match expected {
            None => {
                say!("no expected hash for {bin}");
                failed = true;
            }
            Some(expected_hash) => {
                if !is_executable(&path) {
                    say!("binary {bin} not found at {}", path.display());
                    failed = true;
                    continue;
                }

                let actual = compute_sha256(&path)?;
                if actual != *expected_hash {
                    say!("{bin} hash verification failed:");
                    say!("  expected: {expected_hash}");
                    say!("  actual:   {actual}");
                    failed = true;
                } else {
                    say!("{bin} verified ✓");
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
    let man_url = format!(
        "{release_url}{prefix}_man_{version}.tar.gz",
        prefix = config.network.archive_prefix
    );
    say!("downloading manpages");

    let temp_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => {
            warn!("skipping manpage download: failed to create temp directory");
            return;
        }
    };
    let archive_path = temp_dir.path().join("foundry_man.tar.gz");

    if downloader.download_to_file(&man_url, &archive_path).await.is_err() {
        warn!("skipping manpage download: unavailable or invalid archive");
        return;
    }

    if let Err(e) = extract_tar_gz(&archive_path, &config.man_dir) {
        warn!("skipping manpage download: {e}");
    }
}

pub(crate) fn list(config: &Config) -> Result<()> {
    let bins = config.network.bins;

    if config.versions_dir.exists() {
        for owner_entry in fs::read_dir(&config.versions_dir)? {
            let owner_entry = owner_entry?;
            let owner_path = owner_entry.path();
            if !owner_path.is_dir() {
                continue;
            }

            let owner_name = owner_entry.file_name();
            let owner_name = owner_name.to_string_lossy();

            for repo_entry in fs::read_dir(&owner_path)? {
                let repo_entry = repo_entry?;
                let repo_path = repo_entry.path();
                if !repo_path.is_dir() {
                    continue;
                }

                let repo_name = repo_entry.file_name();
                let repo_name = repo_name.to_string_lossy();

                for version_entry in fs::read_dir(&repo_path)? {
                    let version_entry = version_entry?;
                    let version_path = version_entry.path();
                    if !version_path.is_dir() {
                        continue;
                    }

                    let version_name = version_entry.file_name();
                    let version_name = version_name.to_string_lossy();

                    tell!("{owner_name}/{repo_name} {version_name}");

                    // A missing or non-runnable binary is a broken install.
                    for bin in bins {
                        let bin_path = version_path.join(bin_name(bin));
                        let v = get_bin_version(&bin_path).wrap_err_with(|| {
                            format!(
                                "{owner_name}/{repo_name} {version_name} is broken: \
                                 failed to run {}",
                                bin_path.display()
                            )
                        })?;
                        tell!("- {v}");
                    }
                    println!();
                }
            }
        }
    } else {
        for bin in bins {
            let bin_path = config.bin_path(bin);
            if bin_path.exists() {
                match get_bin_version(&bin_path) {
                    Ok(v) => tell!("- {v}"),
                    Err(_) => tell!("- {bin} (unknown version)"),
                }
            }
        }
    }

    Ok(())
}

/// Resolves channels and version numbers to a release tag; activates any other
/// value verbatim, so `--use 123abc` looks up `123abc` rather than `v123abc`.
///
/// Without an explicit `--repo`, a version not found under the default repo is
/// looked up across all installed repos, keeping `--use <name>` working for
/// custom-repo builds under `versions/<owner>/<repo>/`.
pub(crate) async fn use_version_resolved(
    config: &Config,
    repo: &str,
    version: &str,
    repo_explicit: bool,
) -> Result<()> {
    let tag = if is_resolvable_use_version(version) {
        let downloader = Downloader::new()?;
        let (_version, tag) = resolve_version_and_tag(&downloader, repo, version).await?;
        tag
    } else {
        version.to_string()
    };

    if config.version_dir(repo, &tag).is_dir() {
        return use_version(config, repo, &tag);
    }

    if !repo_explicit {
        if let Some(found_repo) = find_unique_repo_for_version(config, &tag)? {
            return use_version(config, &found_repo, &tag);
        }
    }

    use_version(config, repo, &tag)
}

/// Finds the version directory named `version` across all installed repos,
/// erroring if more than one repo matches.
fn find_unique_repo_for_version(config: &Config, version: &str) -> Result<Option<String>> {
    if !config.versions_dir.exists() {
        return Ok(None);
    }

    let mut matches = Vec::new();
    for owner_entry in fs::read_dir(&config.versions_dir)? {
        let owner_entry = owner_entry?;
        let owner_path = owner_entry.path();
        if !owner_path.is_dir() {
            continue;
        }
        let owner = owner_entry.file_name().to_string_lossy().to_string();

        for repo_entry in fs::read_dir(&owner_path)? {
            let repo_entry = repo_entry?;
            if !repo_entry.path().is_dir() {
                continue;
            }
            let repo = repo_entry.file_name().to_string_lossy().to_string();

            if config.version_dir(&format!("{owner}/{repo}"), version).is_dir() {
                matches.push(format!("{owner}/{repo}"));
            }
        }
    }

    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.remove(0))),
        _ => {
            matches.sort();
            bail!(
                "version {version} is installed for multiple repos ({}); specify --repo to disambiguate",
                matches.join(", ")
            )
        }
    }
}

/// Whether `version` is a channel or a version that resolves to a tag.
fn is_resolvable_use_version(version: &str) -> bool {
    matches!(version, "latest" | "stable" | "nightly") || looks_like_version_number(version)
}

/// Whether `version` looks like a dotted version number: it starts with a digit
/// and has at least two `.<digit>` groups (e.g. `1.5.0`, `1.5.0-rc1`, but not
/// `1.5`, `v1.5.0`, or `123abc`).
fn looks_like_version_number(version: &str) -> bool {
    let bytes = version.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_digit) {
        return false;
    }
    let dot_digit_groups =
        bytes.windows(2).filter(|w| w[0] == b'.' && w[1].is_ascii_digit()).count();
    dot_digit_groups >= 2
}

pub(crate) fn use_version(config: &Config, repo: &str, version: &str) -> Result<()> {
    let version_dir = config.version_dir(repo, version);

    if !version_dir.exists() {
        bail!("version {version} not installed for {repo}");
    }

    // Preflight all bins before mutating, so a missing one can't leave a mix of
    // old and new binaries active.
    for bin in config.network.bins {
        let src = version_dir.join(bin_name(bin));
        if !src.is_file() {
            bail!("binary {bin} not found in version {version} at {}", src.display());
        }
    }

    crate::process::check_bins_in_use(config)?;
    fs::create_dir_all(&config.bin_dir)?;

    for bin in config.network.bins {
        let bin_name = bin_name(bin);
        let src = version_dir.join(&bin_name);
        let dest = config.bin_path(bin);

        let old_version = if dest.exists() { get_bin_version(&dest).ok() } else { None };

        clear_active_bin(config, bin)?;

        #[cfg(unix)]
        std::os::unix::fs::symlink(&src, &dest)?;
        #[cfg(not(unix))]
        fs::copy(&src, &dest)?;

        let v = get_bin_version(&dest)
            .wrap_err_with(|| format!("failed to run {bin} after activating version {version}"))?;
        match old_version {
            Some(old) if old != v => say!("use - {v} (from {old})"),
            _ => say!("use - {v}"),
        }

        if let Ok(which_path) = which::which(bin) {
            if which_path != dest {
                warn!("");
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

/// Resolves a requested version string to its `(version, tag)` pair, where `tag`
/// is the concrete GitHub release tag to download from and `version` is the name
/// used for the archive files and asset names.
///
/// The `latest` and `stable` channels are resolved to the newest non-prerelease
/// tag via the `releases/latest` web redirect, falling back to the GitHub API.
/// The `nightly` channel is resolved to the newest nightly release tag.
/// Everything else is normalized offline.
pub(crate) async fn resolve_version_and_tag(
    downloader: &Downloader,
    repo: &str,
    version: &str,
) -> Result<(String, String)> {
    if version == "latest" || version == "stable" {
        say!("fetching latest release tag from {repo}...");
        let tag = fetch_latest_release_tag(downloader, repo).await?;
        say!("resolved release tag: {tag}");
        Ok((tag.clone(), tag))
    } else if version == "nightly" {
        say!("fetching latest nightly release tags from {repo}...");
        let tag = fetch_latest_nightly_release_tag(downloader, repo).await?;
        say!("resolved nightly release tag: {tag}");
        Ok(("nightly".to_string(), tag))
    } else {
        Ok(normalize_version(version))
    }
}

/// Fetches the newest non-prerelease release tag for `repo`.
///
/// Prefers the `releases/latest` web redirect, which is not subject to the
/// unauthenticated GitHub API rate limit (the cause of CI `403` failures), and
/// falls back to the GitHub API when the redirect cannot be resolved.
async fn fetch_latest_release_tag(downloader: &Downloader, repo: &str) -> Result<String> {
    if let Some(tag) = fetch_latest_release_tag_via_redirect(downloader, repo).await {
        return Ok(tag);
    }
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let body = downloader
        .download_to_string(&url)
        .await
        .wrap_err("failed to fetch release tags from GitHub API")?;
    let json: serde_json::Value = serde_json::from_str(&body)?;
    let tag = json["tag_name"].as_str().filter(|s| !s.is_empty());
    match tag {
        Some(tag) => Ok(tag.to_string()),
        None => bail!("could not find a latest release tag for {repo}"),
    }
}

/// Resolves the latest release tag by following the `releases/latest` web
/// redirect and reading the tag from the final `releases/tag/<tag>` URL.
///
/// Returns `None` (so the caller falls back to the API) when the redirect cannot
/// be resolved or the final URL does not yield a valid tag.
async fn fetch_latest_release_tag_via_redirect(
    downloader: &Downloader,
    repo: &str,
) -> Option<String> {
    let url = format!("https://github.com/{repo}/releases/latest");
    let final_url = downloader.resolve_redirect_url(&url).await.ok()?;
    tag_from_release_url(&final_url)
}

/// Extracts and validates a release tag from a `.../releases/tag/<tag>` URL.
///
/// Returns `None` for any URL that is not a tag URL or whose tag is not a
/// plausible version tag: it must look like `v<digit>...` and contain only
/// `[A-Za-z0-9._-]`, so a trailing slash, query, or fragment is rejected.
pub(crate) fn tag_from_release_url(url: &str) -> Option<String> {
    let (_, tag) = url.rsplit_once("/releases/tag/")?;
    let valid = tag.starts_with('v')
        && tag.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
        && tag.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    valid.then(|| tag.to_string())
}

/// Fetches the newest nightly release tag for `repo` via the GitHub API.
async fn fetch_latest_nightly_release_tag(downloader: &Downloader, repo: &str) -> Result<String> {
    let url = format!("https://api.github.com/repos/{repo}/releases");
    let body = downloader
        .download_to_string(&url)
        .await
        .wrap_err("failed to fetch release tags from GitHub API")?;
    latest_nightly_release_tag(&body, repo)
}

fn latest_nightly_release_tag(json: &str, repo: &str) -> Result<String> {
    let json: serde_json::Value = serde_json::from_str(json)?;
    let releases =
        json.as_array().ok_or_else(|| eyre::eyre!("invalid release tags response for {repo}"))?;

    releases
        .iter()
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            let published_at = release["published_at"].as_str()?;
            tag.starts_with("nightly-").then_some((published_at, tag))
        })
        .max_by_key(|(published_at, _tag)| *published_at)
        .map(|(_published_at, tag)| tag.to_string())
        .ok_or_else(|| eyre::eyre!("could not find a nightly release tag for {repo}"))
}

fn normalize_version(version: &str) -> (String, String) {
    // Only concrete `nightly-<sha>` tags map to the nightly channel here; the
    // bare `nightly` channel is resolved earlier.
    if version.starts_with("nightly-") {
        ("nightly".to_string(), version.to_string())
    } else if version.starts_with(|c: char| c.is_ascii_digit()) {
        let s = format!("v{version}");
        (s.clone(), s)
    } else {
        (version.to_string(), version.to_string())
    }
}

fn bin_name(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

/// Fails with a clear error if `cmd` is not available on `PATH`.
fn need_cmd(cmd: &str) -> Result<()> {
    which::which(cmd).map(|_| ()).map_err(|_| eyre::eyre!("need '{cmd}' (command not found)"))
}

/// Removes any previously activated `bin` from the bin dir, clearing both the
/// bare and `.exe` names. On Windows the active name is `<bin>.exe`, so clearing
/// only that leaves a stale bare `<bin>` (e.g. from the legacy bash foundryup)
/// that could shadow the new one; on Unix the `.exe` removal is a no-op.
fn clear_active_bin(config: &Config, bin: &str) -> Result<()> {
    remove_if_exists(&config.bin_dir.join(bin))?;
    remove_if_exists(&config.bin_dir.join(format!("{bin}.exe")))?;
    Ok(())
}

/// Removes `path` if it exists, including dangling symlinks.
///
/// Uses `symlink_metadata` so a broken symlink is still detected and removed;
/// `Path::exists()` follows symlinks and reports `false` for one, which would
/// leave it in place and make a following `symlink`/`copy` fail.
fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => fs::remove_file(path).map_err(Into::into),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Returns whether `path` is an existing file that is executable.
///
/// On Unix this requires at least one executable permission bit. On Windows
/// executability is not a permission bit, so existence is sufficient.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn get_bin_version(path: &Path) -> Result<String> {
    let output = std::process::Command::new(path).arg("-V").output()?;
    if !output.status.success() {
        bail!("`{} -V` exited with {}", path.display(), output.status);
    }
    let version = String::from_utf8_lossy(&output.stdout);
    Ok(version.trim().to_string())
}

fn rustflags() -> String {
    // Treat an empty `RUSTFLAGS` the same as unset.
    std::env::var("RUSTFLAGS")
        .ok()
        .filter(|flags| !flags.is_empty())
        .unwrap_or_else(|| "-C target-cpu=native".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_nightly_release_tag_selects_newest_published_at() {
        let json = r#"[
            {"tag_name": "nightly-old", "published_at": "2026-05-01T00:00:00Z"},
            {"tag_name": "v1.5.0", "published_at": "2026-06-01T00:00:00Z"},
            {"tag_name": "nightly-new", "published_at": "2026-05-03T00:00:00Z"},
            {"tag_name": "nightly-middle", "published_at": "2026-05-02T00:00:00Z"}
        ]"#;

        let tag = latest_nightly_release_tag(json, "foundry-rs/foundry").unwrap();

        assert_eq!(tag, "nightly-new");
    }

    #[test]
    fn latest_nightly_release_tag_ignores_non_nightly_releases() {
        let json = r#"[
            {"tag_name": "v2.0.0", "published_at": "2026-06-01T00:00:00Z"},
            {"tag_name": "stable", "published_at": "2026-06-02T00:00:00Z"},
            {"tag_name": "nightly-abc", "published_at": "2026-05-01T00:00:00Z"}
        ]"#;

        let tag = latest_nightly_release_tag(json, "foundry-rs/foundry").unwrap();

        assert_eq!(tag, "nightly-abc");
    }

    #[test]
    fn latest_nightly_release_tag_errors_when_no_nightly_exists() {
        let json = r#"[
            {"tag_name": "v2.0.0", "published_at": "2026-06-01T00:00:00Z"}
        ]"#;

        let err = latest_nightly_release_tag(json, "foundry-rs/foundry").unwrap_err();

        assert!(err.to_string().contains("could not find a nightly release tag"));
    }

    #[test]
    fn tag_from_release_url_extracts_valid_tag() {
        // The `releases/latest` redirect lands on a concrete tag page.
        assert_eq!(
            tag_from_release_url("https://github.com/foundry-rs/foundry/releases/tag/v1.7.1"),
            Some("v1.7.1".to_string())
        );
        // `-` and `.` are allowed (e.g. release candidates).
        assert_eq!(
            tag_from_release_url("https://github.com/foundry-rs/foundry/releases/tag/v1.0.0-rc.1"),
            Some("v1.0.0-rc.1".to_string())
        );
    }

    #[test]
    fn tag_from_release_url_rejects_non_tag_or_invalid() {
        // Redirect did not move to a tag page (e.g. landed back on `latest`).
        assert_eq!(
            tag_from_release_url("https://github.com/foundry-rs/foundry/releases/latest"),
            None
        );
        // Tag is not a plausible version (must start with `v<digit>`).
        assert_eq!(
            tag_from_release_url(
                "https://github.com/foundry-rs/foundry/releases/tag/not-a-version"
            ),
            None
        );
        // A trailing slash introduces a disallowed character.
        assert_eq!(
            tag_from_release_url("https://github.com/foundry-rs/foundry/releases/tag/v1.7.1/"),
            None
        );
        // Empty tag.
        assert_eq!(
            tag_from_release_url("https://github.com/foundry-rs/foundry/releases/tag/"),
            None
        );
    }

    #[test]
    fn looks_like_version_number_cases() {
        assert!(looks_like_version_number("1.5.0"));
        assert!(looks_like_version_number("1.5.0-rc1"));
        assert!(looks_like_version_number("10.20.30"));
        // Non-semver tags with two `.<digit>` groups still match.
        assert!(looks_like_version_number("1.2.3foo"));
        // Extra dots/segments are absorbed as long as two `.<digit>` groups exist.
        assert!(looks_like_version_number("1..2.3"));
        assert!(looks_like_version_number("1.a.2.3"));
        // Fewer than two dot-digit groups, or not starting with a digit.
        assert!(!looks_like_version_number("1.5"));
        assert!(!looks_like_version_number("1..2"));
        assert!(!looks_like_version_number("1.a.2"));
        assert!(!looks_like_version_number("v1.5.0"));
        assert!(!looks_like_version_number("123abc"));
        assert!(!looks_like_version_number("nightly"));
        assert!(!looks_like_version_number(""));
    }

    #[test]
    fn is_resolvable_use_version_cases() {
        assert!(is_resolvable_use_version("latest"));
        assert!(is_resolvable_use_version("stable"));
        assert!(is_resolvable_use_version("nightly"));
        assert!(is_resolvable_use_version("1.5.0"));
        assert!(!is_resolvable_use_version("v1.5.0"));
        assert!(!is_resolvable_use_version("foundry-rs-branch-master"));
        assert!(!is_resolvable_use_version("nightly-abc123"));
    }

    #[test]
    fn normalize_version_cases() {
        // `nightly-<sha>` -> asset version "nightly", tag is the literal sha tag.
        assert_eq!(
            normalize_version("nightly-abc123"),
            ("nightly".to_string(), "nightly-abc123".to_string())
        );
        // Bare semver gets a `v` prefix for both asset version and tag.
        assert_eq!(normalize_version("1.5.0"), ("v1.5.0".to_string(), "v1.5.0".to_string()));
        assert_eq!(normalize_version("v1.5.0"), ("v1.5.0".to_string(), "v1.5.0".to_string()));
        // Arbitrary `nightly*` strings are not the nightly channel.
        assert_eq!(
            normalize_version("nightlyfoo"),
            ("nightlyfoo".to_string(), "nightlyfoo".to_string())
        );
        // Branch/custom names pass through untouched.
        assert_eq!(
            normalize_version("foundry-rs-branch-master"),
            ("foundry-rs-branch-master".to_string(), "foundry-rs-branch-master".to_string())
        );
    }

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

    fn test_config(foundry_dir: &Path) -> Config {
        Config {
            foundry_dir: foundry_dir.to_path_buf(),
            versions_dir: foundry_dir.join("versions"),
            bin_dir: foundry_dir.join("bin"),
            man_dir: foundry_dir.join("share/man/man1"),
            network: crate::config::NetworkConfig::FOUNDRY,
        }
    }

    #[test]
    fn clear_active_bin_removes_bare_and_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(tmp.path());
        fs::create_dir_all(&config.bin_dir).unwrap();

        // Simulate a stale bare `forge` (e.g. from the legacy bash foundryup)
        // alongside a stale `forge.exe`.
        fs::write(config.bin_dir.join("forge"), b"stale").unwrap();
        fs::write(config.bin_dir.join("forge.exe"), b"stale").unwrap();

        clear_active_bin(&config, "forge").unwrap();

        assert!(!config.bin_dir.join("forge").exists(), "bare forge should be removed");
        assert!(!config.bin_dir.join("forge.exe").exists(), "forge.exe should be removed");
    }

    #[test]
    fn clear_active_bin_ok_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(tmp.path());
        fs::create_dir_all(&config.bin_dir).unwrap();

        // Nothing to remove: clearing must still succeed.
        clear_active_bin(&config, "forge").unwrap();
    }
}
