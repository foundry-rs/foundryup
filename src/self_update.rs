use crate::{
    config::{Config, FOUNDRYUP_REPO, VERSION},
    download::Downloader,
    platform::Target,
    say,
};
use eyre::Result;
use fs_err as fs;
use semver::Version;

pub(crate) async fn run(config: &Config) -> Result<()> {
    say("updating foundryup...");

    let downloader = Downloader::new()?;
    let new_version = match check_for_update(config).await? {
        Some(v) => v,
        None => {
            say(&format!("foundryup is already up to date (installed: {VERSION})"));
            return Ok(());
        }
    };

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

    downloader.download_to_file(&download_url, &temp_path).await?;

    let foundryup_path = config.foundryup_path();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o755))?;
    }

    self_replace(&temp_path, &foundryup_path)?;

    say(&format!("successfully updated foundryup: {VERSION} → {new_version}"));

    Ok(())
}

pub(crate) async fn check_for_update(_config: &Config) -> Result<Option<String>> {
    let downloader = Downloader::new()?;

    let releases_url = format!("https://api.github.com/repos/{FOUNDRYUP_REPO}/releases/latest");

    let response = match downloader.download_to_string(&releases_url).await {
        Ok(r) => r,
        Err(e) => {
            debug!("failed to check for updates: {e}");
            return Ok(None);
        }
    };

    let json: serde_json::Value = serde_json::from_str(&response)?;
    let tag_name =
        json["tag_name"].as_str().ok_or_else(|| eyre::eyre!("missing tag_name in release"))?;

    let remote_version = tag_name.trim_start_matches('v');

    let current = Version::parse(VERSION)?;
    let remote = match Version::parse(remote_version) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };

    if remote > current { Ok(Some(remote_version.to_string())) } else { Ok(None) }
}

fn self_replace(src: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::rename(src, dest)?;
        Ok(())
    }

    #[cfg(windows)]
    {
        let current_exe = std::env::current_exe()?;
        let backup_path = current_exe.with_extension("old.exe");

        if backup_path.exists() {
            fs::remove_file(&backup_path).ok();
        }

        fs::rename(&current_exe, &backup_path)?;
        fs::copy(src, dest)?;

        Ok(())
    }
}
