//! Tests that exercise `check_bins_in_use`, which scans for running
//! `forge`/`cast`/`anvil`/`chisel` processes globally and also spawns those
//! binaries themselves. They must not run concurrently, so they live in this
//! module to be matched by a single nextest filter (see `.config/nextest.toml`).

use super::*;
use std::path::Path;

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

fn test_install(version: &str) {
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
        assert!(foundry_dir.join("bin").join(&name).exists(), "{name} does not exist");
    }

    run_forge_test(&foundry_dir, temp_dir.path());

    foundryup().env("FOUNDRY_DIR", &foundry_dir).arg("--list").assert().success().stderr_eq(str![
        [r#"
foundryup: foundry-rs/foundry [..]
foundryup: - forge [..]
foundryup: - cast [..]
foundryup: - anvil [..]
foundryup: - chisel [..]

...
"#]
    ]);
}

#[test]
fn install_stable() {
    test_install("stable");
}
#[test]
fn install_nightly() {
    test_install("nightly");
}
#[test]
fn install_v1_5_0() {
    test_install("v1.5.0");
}
#[test]
fn install_1_5_0() {
    test_install("1.5.0");
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
