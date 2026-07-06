use crate::say;
use eyre::Result;
use fs_err as fs;
use std::path::{Path, PathBuf};

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("VERGEN_GIT_SHA"),
    " ",
    env!("VERGEN_BUILD_TIMESTAMP"),
    ")"
);

pub(crate) const FOUNDRYUP_REPO: &str = "foundry-rs/foundryup";

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).filter(|value| !value.is_empty()).map(PathBuf::from)
}

#[derive(Debug)]
pub(crate) struct Config {
    pub foundry_dir: PathBuf,
    pub versions_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub man_dir: PathBuf,
    pub network: NetworkConfig,
}

impl Config {
    pub(crate) fn new() -> Result<Self> {
        let base_dir = env_path("XDG_CONFIG_HOME").or_else(home::home_dir);

        let base_dir = base_dir.ok_or_else(|| eyre::eyre!("could not determine home directory"))?;

        let foundry_dir = env_path("FOUNDRY_DIR").unwrap_or_else(|| base_dir.join(".foundry"));

        let versions_dir = foundry_dir.join("versions");
        let bin_dir = foundry_dir.join("bin");
        let man_dir = foundry_dir.join("share/man/man1");
        Ok(Self { foundry_dir, versions_dir, bin_dir, man_dir, network: NetworkConfig::FOUNDRY })
    }

    pub(crate) fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.versions_dir)?;
        fs::create_dir_all(&self.bin_dir)?;
        fs::create_dir_all(&self.man_dir)?;
        Ok(())
    }

    pub(crate) fn migrate_legacy_versions(&self) -> Result<()> {
        if !self.versions_dir.exists() {
            return Ok(());
        }

        let default_repo = NetworkConfig::FOUNDRY.repo;

        for entry in fs::read_dir(&self.versions_dir)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            let name = entry.file_name();
            let name = name.to_string_lossy();

            if name.contains('/') || self.is_owner_dir(&path) {
                continue;
            }

            if self.is_legacy_version_dir(&path) {
                let new_path = self.version_dir(default_repo, &name);
                if new_path.exists() {
                    continue;
                }
                fs::create_dir_all(new_path.parent().unwrap())?;
                say!("migrating legacy version '{name}' to {default_repo}/{name}");
                fs::rename(&path, &new_path)?;
            }
        }

        Ok(())
    }

    fn is_legacy_version_dir(&self, path: &Path) -> bool {
        for &(_, bin) in NetworkConfig::FOUNDRY.bins {
            let bin_name = if cfg!(windows) { format!("{bin}.exe") } else { bin.to_string() };
            if path.join(&bin_name).exists() {
                return true;
            }
        }
        false
    }

    fn is_owner_dir(&self, path: &Path) -> bool {
        // Owner dirs have repo subdirs, which have version subdirs.
        fn has_dir(path: &Path, mut f: impl FnMut(&Path) -> bool) -> bool {
            fs::read_dir(path)
                .into_iter()
                .flatten()
                .flatten()
                .any(|entry| f(&entry.path()) && entry.metadata().is_ok_and(|m| m.is_dir()))
        }
        has_dir(path, |p| has_dir(p, |_| true))
    }

    pub(crate) fn version_dir(&self, repo: &str, version: &str) -> PathBuf {
        self.versions_dir.join(repo).join(version)
    }

    pub(crate) fn bin_path(&self, name: &str) -> PathBuf {
        let name = if cfg!(windows) && !name.ends_with(".exe") {
            format!("{name}.exe")
        } else {
            name.to_string()
        };
        self.bin_dir.join(name)
    }

    pub(crate) fn repo_dir(&self, repo: &str) -> PathBuf {
        self.foundry_dir.join(repo)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NetworkConfig {
    pub repo: &'static str,
    pub bins: &'static [(bool, &'static str)],
    pub archive_prefix: &'static str,
    pub default_version: &'static str,
    pub display_name: &'static str,
    pub has_attestation: bool,
}

const REQUIRED: bool = false;
const OPTIONAL: bool = true;

impl NetworkConfig {
    pub(crate) const FOUNDRY: Self = Self {
        repo: "foundry-rs/foundry",
        bins: &[
            (REQUIRED, "forge"),
            (REQUIRED, "cast"),
            (REQUIRED, "anvil"),
            (REQUIRED, "chisel"),
            (OPTIONAL, "solar"),
        ],
        archive_prefix: "foundry",
        default_version: "stable",
        display_name: "foundry",
        has_attestation: true,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        ffi::OsString,
        path::Path,
        sync::{Mutex, MutexGuard},
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        _guard: MutexGuard<'static, ()>,
        vars: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvGuard {
        fn new() -> Self {
            let guard = ENV_LOCK.lock().unwrap();
            let vars = ["FOUNDRY_DIR", "XDG_CONFIG_HOME", "HOME", "USERPROFILE"]
                .into_iter()
                .map(|key| (key, env::var_os(key)))
                .collect();
            Self { _guard: guard, vars }
        }

        fn set(&self, key: &str, value: impl AsRef<Path>) {
            // These tests serialize environment access with ENV_LOCK.
            unsafe { env::set_var(key, value.as_ref()) };
        }

        fn set_empty(&self, key: &str) {
            // These tests serialize environment access with ENV_LOCK.
            unsafe { env::set_var(key, "") };
        }

        fn remove(&self, key: &str) {
            // These tests serialize environment access with ENV_LOCK.
            unsafe { env::remove_var(key) };
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.vars.iter().rev() {
                // These tests serialize environment access with ENV_LOCK.
                unsafe {
                    match value {
                        Some(value) => env::set_var(key, value),
                        None => env::remove_var(key),
                    }
                }
            }
        }
    }

    fn assert_foundry_dirs(config: &Config, foundry_dir: &Path) {
        assert_eq!(config.foundry_dir, foundry_dir);
        assert_eq!(config.versions_dir, foundry_dir.join("versions"));
        assert_eq!(config.bin_dir, foundry_dir.join("bin"));
        assert_eq!(config.man_dir, foundry_dir.join("share/man/man1"));
    }

    #[test]
    fn config_falls_back_to_home_without_xdg_config_home() {
        let temp_dir = tempfile::tempdir().unwrap();
        let home_dir = temp_dir.path().join("home");
        let env = EnvGuard::new();
        env.remove("FOUNDRY_DIR");
        env.remove("XDG_CONFIG_HOME");
        env.set("HOME", &home_dir);
        env.set("USERPROFILE", &home_dir);

        let config = Config::new().unwrap();

        assert_foundry_dirs(&config, &home_dir.join(".foundry"));
    }

    #[test]
    fn config_uses_xdg_config_home_before_home() {
        let temp_dir = tempfile::tempdir().unwrap();
        let home_dir = temp_dir.path().join("home");
        let xdg_config_home = temp_dir.path().join("config");
        let env = EnvGuard::new();
        env.remove("FOUNDRY_DIR");
        env.set("XDG_CONFIG_HOME", &xdg_config_home);
        env.set("HOME", &home_dir);
        env.set("USERPROFILE", &home_dir);

        let config = Config::new().unwrap();

        assert_foundry_dirs(&config, &xdg_config_home.join(".foundry"));
    }

    #[test]
    fn config_empty_xdg_config_home_falls_back_to_home() {
        let temp_dir = tempfile::tempdir().unwrap();
        let home_dir = temp_dir.path().join("home");
        let env = EnvGuard::new();
        env.remove("FOUNDRY_DIR");
        env.set_empty("XDG_CONFIG_HOME");
        env.set("HOME", &home_dir);
        env.set("USERPROFILE", &home_dir);

        let config = Config::new().unwrap();

        assert_foundry_dirs(&config, &home_dir.join(".foundry"));
    }

    #[test]
    fn config_empty_foundry_dir_uses_default_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let home_dir = temp_dir.path().join("home");
        let xdg_config_home = temp_dir.path().join("config");
        let env = EnvGuard::new();
        env.set_empty("FOUNDRY_DIR");
        env.set("XDG_CONFIG_HOME", &xdg_config_home);
        env.set("HOME", &home_dir);
        env.set("USERPROFILE", &home_dir);

        let config = Config::new().unwrap();

        assert_foundry_dirs(&config, &xdg_config_home.join(".foundry"));
    }

    #[test]
    fn config_foundry_dir_overrides_base_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let home_dir = temp_dir.path().join("home");
        let xdg_config_home = temp_dir.path().join("config");
        let foundry_dir = temp_dir.path().join("custom-foundry");
        let env = EnvGuard::new();
        env.set("FOUNDRY_DIR", &foundry_dir);
        env.set("XDG_CONFIG_HOME", &xdg_config_home);
        env.set("HOME", &home_dir);
        env.set("USERPROFILE", &home_dir);

        let config = Config::new().unwrap();

        assert_foundry_dirs(&config, &foundry_dir);
    }
}
