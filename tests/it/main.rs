use snapbox::{cmd::Command, str};

fn foundryup() -> Command {
    Command::new(snapbox::cmd::cargo_bin!("foundryup")).env("NO_COLOR", "1")
}

#[test]
fn help() {
    foundryup().arg("--help").assert().success().stdout_eq(str![[r#"
The installer for Foundry.

Update or revert to a specific Foundry version with ease.

By default, the latest stable version is installed from built binaries.

Usage: foundryup [OPTIONS]

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

Usage: foundryup --pr <PR>

For more information, try '--help'.

"#]]);
}

#[test]
fn use_nonexistent_version() {
    let temp_dir =
        tempfile::Builder::new().prefix("foundryup-test-use-nonexistent").tempdir().unwrap();

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
    let temp_dir = tempfile::Builder::new().prefix("foundryup-test-list-empty").tempdir().unwrap();

    foundryup()
        .env("FOUNDRY_DIR", temp_dir.path().join(".foundry"))
        .arg("--list")
        .assert()
        .success();
}

#[test]
fn install_stable() {
    let temp_dir = tempfile::Builder::new().prefix("foundryup-test-stable").tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");

    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .args(["-i", "stable"])
        .assert()
        .success()
        .stderr_eq(str![[r#"
...
[..]done!
...
"#]]);

    assert!(foundry_dir.join("bin/forge").exists());
    assert!(foundry_dir.join("bin/cast").exists());
    assert!(foundry_dir.join("bin/anvil").exists());
    assert!(foundry_dir.join("bin/chisel").exists());
}

#[test]
fn install_nightly() {
    let temp_dir = tempfile::Builder::new().prefix("foundryup-test-nightly").tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");

    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .args(["-i", "nightly"])
        .assert()
        .success()
        .stderr_eq(str![[r#"
...
[..]done!
...
"#]]);

    assert!(foundry_dir.join("bin/forge").exists());
    assert!(foundry_dir.join("bin/cast").exists());
}

#[test]
fn list_after_install() {
    let temp_dir =
        tempfile::Builder::new().prefix("foundryup-test-list-after-install").tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");

    foundryup().env("FOUNDRY_DIR", &foundry_dir).args(["-i", "stable"]).assert().success();

    foundryup().env("FOUNDRY_DIR", &foundry_dir).arg("--list").assert().success().stderr_eq(str![
        [r#"
...
[..]stable
[..]forge [..]
...
"#]
    ]);
}

#[test]
fn use_version() {
    let temp_dir = tempfile::Builder::new().prefix("foundryup-test-use").tempdir().unwrap();
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
    let temp_dir = tempfile::Builder::new().prefix("foundryup-test-cache").tempdir().unwrap();
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
