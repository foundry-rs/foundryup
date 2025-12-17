use crate::{
    config::{Config, FOUNDRYUP_REPO, VERSION},
    download::Downloader,
    platform::Target,
    say,
};
use eyre::{Result, WrapErr};
use fs_err as fs;
use semver::Version;
use tracing::debug;

pub(crate) async fn run(config: &Config) -> Result<()> {
    say("checking for updates...");

    let new_version = match check_for_update(config).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            say(&format!("foundryup is already up to date (installed: {VERSION})"));
            return Ok(());
        }
        Err(e) => {
            debug!("update check failed: {e}");
            return Err(e).wrap_err("failed to check for updates");
        }
    };

    say(&format!("downloading foundryup v{new_version}..."));

    let downloader = Downloader::new()?;
    let target = Target::detect(None, None)?;
    let archive_name = format!(
        "foundryup_{platform}_{arch}",
        platform = target.platform.as_str(),
        arch = target.arch.as_str()
    );

    let download_url = format!(
        "https://github.com/{FOUNDRYUP_REPO}/releases/download/v{new_version}/{archive_name}"
    );

    let temp_dir = tempfile::tempdir()?;
    let temp_path = temp_dir.path().join("foundryup_new");

    downloader
        .download_to_file(&download_url, &temp_path)
        .await
        .wrap_err_with(|| format!("failed to download foundryup v{new_version}"))?;

    say("installing update...");

    self_replace::self_replace(&temp_path).wrap_err("failed to replace foundryup binary")?;

    let _ = fs::remove_file(temp_path);

    let _ = config;

    say(&format!("successfully updated foundryup: {VERSION} → {new_version}"));

    Ok(())
}

pub(crate) async fn check_for_update(_config: &Config) -> Result<Option<String>> {
    let downloader = Downloader::new()?;

    let releases_url = format!("https://api.github.com/repos/{FOUNDRYUP_REPO}/releases/latest");

    debug!("fetching latest release from {releases_url}");

    let response = downloader
        .download_to_string(&releases_url)
        .await
        .wrap_err("failed to fetch release information")?;

    let json: serde_json::Value =
        serde_json::from_str(&response).wrap_err("failed to parse release JSON")?;

    let tag_name = json["tag_name"]
        .as_str()
        .ok_or_else(|| eyre::eyre!("missing tag_name in release response"))?;

    let remote_version = tag_name.trim_start_matches('v');

    debug!("current version: {VERSION}, remote version: {remote_version}");

    let current = Version::parse(VERSION).wrap_err("failed to parse current version")?;
    let remote = match Version::parse(remote_version) {
        Ok(v) => v,
        Err(e) => {
            debug!("failed to parse remote version '{remote_version}': {e}");
            return Ok(None);
        }
    };

    if remote > current { Ok(Some(remote_version.to_string())) } else { Ok(None) }
}
