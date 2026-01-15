use snapbox::{cmd::Command, str};
use std::{env::consts::EXE_SUFFIX, path::Path};

const BINS: &[&str] = &["forge", "cast", "anvil", "chisel"];
const TEMPO_BINS: &[&str] = &["forge", "cast"];

mod installer_script;
mod self_update;

fn foundryup() -> Command {
    Command::new(snapbox::cmd::cargo_bin!("foundryup")).env("NO_COLOR", "1")
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

  -n, --network <NETWORK>
          Install binaries for a specific network (e.g., tempo)
          
          [possible values: tempo]

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

#[test]
fn install_tempo_nightly() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");

    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .args(["--network", "tempo"])
        .assert()
        .success()
        .stderr_eq(str![[r#"
...
[..]installing tempo-foundry[..]
...
[..]done!
...
"#]]);

    for &bin in TEMPO_BINS {
        let name = format!("{bin}{EXE_SUFFIX}");
        assert!(foundry_dir.join("bin").join(&name).exists(), "{name} does not exist");
    }

    run_forge_test(&foundry_dir, temp_dir.path());
}
