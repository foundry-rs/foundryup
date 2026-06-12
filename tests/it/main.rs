use snapbox::{cmd::Command, str};
use std::env::consts::EXE_SUFFIX;

const BINS: &[&str] = &["forge", "cast", "anvil", "chisel"];

mod foundry_bins;
mod installer_script;
mod self_update;

fn foundryup() -> Command {
    Command::new(snapbox::cmd::cargo_bin!("foundryup")).env("NO_COLOR", "1")
}

/// Writes an executable stub binary at `path`.
///
/// Activation now runs `<bin> -V` and fails if it cannot execute, so test
/// fixtures need real executables rather than placeholder text. We reuse the
/// `foundryup` test binary, which exits 0 and prints a version for `-V` on every
/// platform (and copying it preserves the executable bit on Unix).
fn write_fake_bin(path: &std::path::Path) {
    std::fs::copy(snapbox::cmd::cargo_bin!("foundryup"), path).unwrap();
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

  -V, --version
          Print version

  -h, --help
          Print help (see a summary with '-h')

"#]]);
}

#[test]
fn version() {
    foundryup().arg("--version").assert().success().stdout_eq(str![[r#"
foundryup [..]
"#]]);
}

#[test]
fn version_short() {
    foundryup().arg("-V").assert().success().stdout_eq(str![[r#"
foundryup [..]
"#]]);
}

// `-v` is an alias for `--version`.
#[test]
fn version_short_alias() {
    foundryup().arg("-v").assert().success().stdout_eq(str![[r#"
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

// `--use` normalizes a bare semver (e.g. `1.5.0`) to the `v1.5.0` directory.
#[test]
fn use_version_normalizes_bare_semver() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");
    let version_dir = foundry_dir.join("versions/foundry-rs/foundry/v1.5.0");
    std::fs::create_dir_all(&version_dir).unwrap();

    for bin in BINS {
        let bin_path = version_dir.join(format!("{bin}{EXE_SUFFIX}"));
        write_fake_bin(&bin_path);
    }

    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .args(["--use", "1.5.0"])
        .assert()
        .success()
        .stderr_eq(str![[r#"
...
[..]use - [..]
...
"#]]);

    for &bin in BINS {
        let name = format!("{bin}{EXE_SUFFIX}");
        assert!(foundry_dir.join("bin").join(&name).exists(), "{name} was not activated");
    }
}

// `--use` activates a digit-prefixed non-semver name verbatim, without the `v`
// prefix applied to semver versions.
#[test]
fn use_version_digit_prefixed_name_is_literal() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");
    let version_dir = foundry_dir.join("versions/foundry-rs/foundry/123abc");
    std::fs::create_dir_all(&version_dir).unwrap();

    for bin in BINS {
        let bin_path = version_dir.join(format!("{bin}{EXE_SUFFIX}"));
        write_fake_bin(&bin_path);
    }

    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .args(["--use", "123abc"])
        .assert()
        .success()
        .stderr_eq(str![[r#"
...
[..]use - [..]
...
"#]]);

    for &bin in BINS {
        let name = format!("{bin}{EXE_SUFFIX}");
        assert!(foundry_dir.join("bin").join(&name).exists(), "{name} was not activated");
    }
}

// On unix, `--use` activates a version by symlinking it into the bin dir.
#[cfg(unix)]
#[test]
fn use_version_creates_symlink_on_unix() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");
    let version_dir = foundry_dir.join("versions/foundry-rs/foundry/v1.5.0");
    std::fs::create_dir_all(&version_dir).unwrap();

    for bin in BINS {
        write_fake_bin(&version_dir.join(bin));
    }

    foundryup().env("FOUNDRY_DIR", &foundry_dir).args(["--use", "1.5.0"]).assert().success();

    for &bin in BINS {
        let dest = foundry_dir.join("bin").join(bin);
        let meta = std::fs::symlink_metadata(&dest).unwrap();
        assert!(meta.file_type().is_symlink(), "{bin} should be a symlink");
        assert_eq!(std::fs::read_link(&dest).unwrap(), version_dir.join(bin));
    }
}

// Empty `FOUNDRYUP_*` env vars are treated as unset, so an empty `FOUNDRYUP_JOBS`
// must not fail clap's `u32` parsing.
#[test]
fn empty_foundryup_env_is_ignored() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();

    foundryup()
        .env("FOUNDRY_DIR", temp_dir.path().join(".foundry"))
        .env("FOUNDRYUP_JOBS", "")
        .env("FOUNDRYUP_VERSION", "")
        .env("FOUNDRYUP_PR", "")
        .arg("--list")
        .assert()
        .success();
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
