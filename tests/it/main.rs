use snapbox::{cmd::Command, str};
use std::env::consts::EXE_SUFFIX;

const BINS: &[&str] = &["forge", "cast", "anvil", "chisel"];

mod foundry_bins;
mod installer_script;
mod self_update;

fn foundryup() -> Command {
    Command::new(snapbox::cmd::cargo_bin!("foundryup")).env("NO_COLOR", "1")
}

#[test]
fn help() {
    foundryup().arg("--help").assert().success().stdout_eq(str![[r#"
The installer for Foundry.

Update or revert to a specific Foundry version with ease.

By default, the latest stable version is installed from built binaries.

Usage: foundryup[EXE] [OPTIONS]

Options:
  -U, --update
          Update foundryup to the latest version

  -r, --repo <REPO>
          Build and install from a remote GitHub repo (uses default branch if no other options)

  -b, --branch <BRANCH>
          Build and install a specific branch

  -i, --install <VERSION>
          Install a specific version from built binaries (e.g., stable, nightly, 0.3.0)

  -l, --list
          List installed versions

  -u, --use <VERSION>
          Use a specific installed version

  -p, --path <PATH>
          Build and install a local repository

  -P, --pr <PR>
          Build and install a specific Pull Request

  -C, --commit <COMMIT>
          Build and install a specific commit

  -j, --jobs <JOBS>
          Number of CPUs to use for building (default: all)

      --cargo-profile <CARGO_PROFILE>
          Cargo profile to use for building
          
          [default: release]

      --cargo-features <CARGO_FEATURES>
          Cargo features to enable for building

  -f, --force
          Skip SHA verification (INSECURE)

      --arch <ARCH>
          Install a specific architecture (amd64, arm64)

      --platform <PLATFORM>
          Install a specific platform (win32, linux, darwin, alpine)

      --completions <SHELL>
          Generate shell completions
          
          [possible values: bash, elvish, fish, powershell, zsh]

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

"#]]);
}

#[test]
fn version() {
    foundryup().arg("--version").assert().success().stdout_eq(str![[r#"
foundryup [..]
"#]]);
}

#[test]
fn completions_bash() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_foundryup"))
        .args(["--completions", "bash"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("_foundryup"), "expected _foundryup in completions");
}

#[test]
fn conflicting_args() {
    foundryup().args(["--pr", "123", "--branch", "main"]).assert().failure().stderr_eq(str![[r#"
error: the argument '--pr <PR>' cannot be used with '--branch <BRANCH>'

Usage: foundryup[EXE] --pr <PR>

For more information, try '--help'.

"#]]);
}

#[test]
fn use_nonexistent_version() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();

    foundryup()
        .env("FOUNDRY_DIR", temp_dir.path().join(".foundry"))
        .args(["--use", "nonexistent-version"])
        .assert()
        .failure()
        .stderr_eq(str![[r#"
...
[..]version nonexistent-version not installed[..]
...
"#]]);
}

#[test]
fn list_empty() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();

    foundryup()
        .env("FOUNDRY_DIR", temp_dir.path().join(".foundry"))
        .arg("--list")
        .assert()
        .success();
}

#[test]
fn migrate_legacy_versions() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");
    let versions_dir = foundry_dir.join("versions");

    std::fs::create_dir_all(versions_dir.join("nightly")).unwrap();
    std::fs::create_dir_all(versions_dir.join("stable")).unwrap();

    for version in ["nightly", "stable"] {
        for bin in BINS {
            let bin_path = versions_dir.join(version).join(format!("{bin}{EXE_SUFFIX}"));
            std::fs::write(&bin_path, "fake binary").unwrap();
        }
    }

    assert!(versions_dir.join("nightly").exists());
    assert!(versions_dir.join("stable").exists());

    foundryup().env("FOUNDRY_DIR", &foundry_dir).arg("--list").assert().success().stderr_eq(str![
        [r#"
...
foundryup: migrating legacy version [..]
...
foundryup: migrating legacy version [..]
...
"#]
    ]);

    assert!(!versions_dir.join("nightly").exists());
    assert!(!versions_dir.join("stable").exists());
    assert!(versions_dir.join("foundry-rs/foundry/nightly").exists());
    assert!(versions_dir.join("foundry-rs/foundry/stable").exists());
}
