//! Tests that exercise `check_bins_in_use`, which scans for running Foundry
//! processes globally and also spawns the required binaries themselves. They
//! must not run concurrently, so they live in this module to be matched by a
//! single nextest filter (see `.config/nextest.toml`).

use super::*;
use std::path::{Path, PathBuf};

fn active_bin_path(foundry_dir: &Path, bin: &str) -> PathBuf {
    foundry_dir.join("bin").join(format!("{bin}{EXE_SUFFIX}"))
}

fn list_output(foundry_dir: &Path) -> String {
    let assert = foundryup().env("FOUNDRY_DIR", foundry_dir).arg("--list").assert().success();
    String::from_utf8_lossy(&assert.get_output().stdout).into_owned()
}

fn assert_optional_bins(foundry_dir: &Path, stdout: &str, expected: &[&str]) {
    for &bin in OPTIONAL_BINS {
        let expected = expected.contains(&bin);
        let name = format!("{bin}{EXE_SUFFIX}");
        assert_eq!(
            active_bin_path(foundry_dir, bin).exists(),
            expected,
            "{name} activation mismatch"
        );
        assert_eq!(
            stdout.contains(&format!("foundryup: - {bin} ")),
            expected,
            "{bin} list output mismatch"
        );
    }
}

fn run_forge_test(foundry_dir: &Path, temp_dir: &Path) {
    let forge = foundry_dir.join(format!("bin/forge{EXE_SUFFIX}"));

    Command::new(&forge).arg("--version").assert().success().stdout_eq(str![[r#"
forge [..]
...
"#]]);

    Command::new(&forge).args(["init", "test-project"]).current_dir(temp_dir).assert().success();
    let project_dir = temp_dir.join("test-project");

    Command::new(&forge).arg("test").current_dir(&project_dir).assert().success();
}

fn test_install(version: &str, expected_optional_bins: Option<&[&str]>) {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");

    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .args(["-i", version])
        .assert()
        .success()
        .stderr_eq(str![[r#"
...
[..]done!
...
"#]]);

    for &bin in BINS {
        let name = format!("{bin}{EXE_SUFFIX}");
        assert!(active_bin_path(&foundry_dir, bin).exists(), "{name} does not exist");
    }

    run_forge_test(&foundry_dir, temp_dir.path());

    foundryup().env("FOUNDRY_DIR", &foundry_dir).arg("--list").assert().success().stdout_eq(str![
        [r#"
foundryup: foundry-rs/foundry [..]
foundryup: - forge [..]
foundryup: - cast [..]
foundryup: - anvil [..]
foundryup: - chisel [..]
...
"#]
    ]);

    if let Some(expected_optional_bins) = expected_optional_bins {
        let stdout = list_output(&foundry_dir);
        assert_optional_bins(&foundry_dir, &stdout, expected_optional_bins);
    }
}

#[test]
fn install_stable() {
    test_install("stable", None);
}
// `latest` resolves to the newest non-prerelease tag via the GitHub API.
#[test]
fn install_latest() {
    test_install("latest", None);
}
#[test]
fn install_nightly() {
    test_install("nightly", Some(OPTIONAL_BINS));
}
#[test]
fn install_v1_7_0() {
    test_install("v1.7.0", Some(&[]));
}
#[test]
fn install_v1_5_0() {
    test_install("v1.5.0", None);
}
#[test]
fn install_1_5_0() {
    test_install("1.5.0", None);
}

#[test]
fn use_version() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");

    foundryup().env("FOUNDRY_DIR", &foundry_dir).args(["-i", "stable"]).assert().success();

    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .args(["--use", "stable"])
        .assert()
        .success()
        .stderr_eq(str![[r#"
...
[..]use - forge [..]
...
"#]]);
}

#[test]
fn reinstall_uses_cache() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");

    foundryup().env("FOUNDRY_DIR", &foundry_dir).args(["-i", "stable"]).assert().success();

    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .args(["-i", "stable"])
        .assert()
        .success()
        .stderr_eq(str![[r#"
...
[..]already installed and verified[..]
...
[..]done!
...
"#]]);
}
