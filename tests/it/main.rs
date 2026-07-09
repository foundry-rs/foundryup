use snapbox::{assert_data_eq, cmd::Command, str};
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
/// platform. Linking avoids copying the debug test binary for every fake bin.
fn write_fake_bin(path: &std::path::Path) {
    let source = snapbox::cmd::cargo_bin!("foundryup");
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, path).unwrap();
    #[cfg(windows)]
    std::fs::hard_link(source, path).unwrap();
    #[cfg(not(any(unix, windows)))]
    std::fs::copy(source, path).unwrap();
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
    assert_data_eq!(
        stdout.as_ref(),
        str![[r#"
...
_foundryup()[..]
...

"#]]
    );
}

#[test]
fn conflicting_args() {
    foundryup().args(["--pr", "123", "--branch", "main"]).assert().failure().stderr_eq(str![[r#"
error: the argument '--pr <PR>' cannot be used with '--branch <BRANCH>'

Usage: foundryup[EXE] --pr <PR>

For more information, try '--help'.

"#]]);
}

// `--update` is handled before the `--list`/`--use` command modes, so combining
// them must error at parse time instead of silently self-updating.
#[test]
fn update_conflicts_with_list_and_use() {
    foundryup().args(["--list", "--update"]).assert().failure().stderr_eq(str![[r#"
error: the argument '--list' cannot be used with '--update'

Usage: foundryup[EXE] --list

For more information, try '--help'.

"#]]);

    foundryup().args(["--use", "v1.0.0", "--update"]).assert().failure().stderr_eq(str![[r#"
error: the argument '--use <VERSION>' cannot be used with '--update'

Usage: foundryup[EXE] --use <VERSION>

For more information, try '--help'.

"#]]);
}

// An empty `--use` value is rejected, not defaulted (matching the shell installer).
#[test]
fn use_empty_version_errors() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();

    foundryup()
        .env("FOUNDRY_DIR", temp_dir.path().join(".foundry"))
        .args(["--use", ""])
        .assert()
        .failure()
        .stderr_eq(str![[r#"
...
[..]no version provided[..]
...
"#]]);
}

// An empty option value (here `--repo ""`) is treated as unset and falls back to
// its default, so activation looks under the default `foundry-rs/foundry` repo.
#[test]
fn empty_repo_falls_back_to_default() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();

    foundryup()
        .env("FOUNDRY_DIR", temp_dir.path().join(".foundry"))
        .args(["--repo", "", "--use", "nonexistent-version"])
        .assert()
        .failure()
        .stderr_eq(str![[r#"
...
[..]version nonexistent-version not installed for foundry-rs/foundry[..]
...
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

// `--use <name>` without `--repo` finds a build installed under a custom repo.
#[test]
fn use_version_finds_custom_repo_without_repo_flag() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");
    let version_dir = foundry_dir.join("versions/someone/foundry/someone-branch-x");
    std::fs::create_dir_all(&version_dir).unwrap();

    for bin in BINS {
        let bin_path = version_dir.join(format!("{bin}{EXE_SUFFIX}"));
        write_fake_bin(&bin_path);
    }

    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .args(["--use", "someone-branch-x"])
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

// `--use <name>` errors when the same version name exists under multiple repos.
#[test]
fn use_version_ambiguous_across_repos_errors() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");

    for repo in ["alice/foundry", "bob/foundry"] {
        let version_dir = foundry_dir.join(format!("versions/{repo}/shared-name"));
        std::fs::create_dir_all(&version_dir).unwrap();
        for bin in BINS {
            write_fake_bin(&version_dir.join(format!("{bin}{EXE_SUFFIX}")));
        }
    }

    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .args(["--use", "shared-name"])
        .assert()
        .failure()
        .stderr_eq(str![[r#"
...
[..]installed for multiple repos[..]
...
"#]]);
}

// A `--use` that fails on a missing binary must not leave a partially-switched
// toolchain: the previously active version stays fully active.
#[cfg(unix)]
#[test]
fn use_version_missing_bin_leaves_previous_active() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");
    let good_dir = foundry_dir.join("versions/foundry-rs/foundry/v1.0.0");
    let broken_dir = foundry_dir.join("versions/foundry-rs/foundry/v2.0.0");
    std::fs::create_dir_all(&good_dir).unwrap();
    std::fs::create_dir_all(&broken_dir).unwrap();

    // v1.0.0 is complete; v2.0.0 is missing `chisel`.
    for bin in BINS {
        write_fake_bin(&good_dir.join(format!("{bin}{EXE_SUFFIX}")));
    }
    for bin in BINS.iter().filter(|b| **b != "chisel") {
        write_fake_bin(&broken_dir.join(format!("{bin}{EXE_SUFFIX}")));
    }

    // Activate the good version.
    foundryup().env("FOUNDRY_DIR", &foundry_dir).args(["--use", "1.0.0"]).assert().success();

    // Attempting to switch to the broken version fails...
    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .args(["--use", "2.0.0"])
        .assert()
        .failure()
        .stderr_eq(str![[r#"
...
[..]binary chisel not found[..]
...
"#]]);

    // ...and every active binary must still point at v1.0.0, not a mix.
    let bin_dir = foundry_dir.join("bin");
    for &bin in BINS {
        let active = bin_dir.join(format!("{bin}{EXE_SUFFIX}"));
        let target = std::fs::read_link(&active)
            .unwrap_or_else(|e| panic!("{bin} is not an active symlink: {e}"));
        assert_eq!(target, good_dir.join(format!("{bin}{EXE_SUFFIX}")), "{bin} was switched");
    }
}

// A `--use` that fails on a broken (present but non-runnable) binary must also
// leave the previously active version fully active, not a partial switch.
#[cfg(unix)]
#[test]
fn use_version_broken_bin_leaves_previous_active() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");
    let good_dir = foundry_dir.join("versions/foundry-rs/foundry/v1.0.0");
    let broken_dir = foundry_dir.join("versions/foundry-rs/foundry/v2.0.0");
    std::fs::create_dir_all(&good_dir).unwrap();
    std::fs::create_dir_all(&broken_dir).unwrap();

    // v1.0.0 is complete; v2.0.0 has all files but `chisel` is not runnable.
    for bin in BINS {
        write_fake_bin(&good_dir.join(format!("{bin}{EXE_SUFFIX}")));
    }
    for bin in BINS.iter().filter(|b| **b != "chisel") {
        write_fake_bin(&broken_dir.join(format!("{bin}{EXE_SUFFIX}")));
    }
    // A present-but-non-executable file: exists, but fails to run.
    std::fs::write(broken_dir.join(format!("chisel{EXE_SUFFIX}")), b"not an executable").unwrap();

    // Activate the good version.
    foundryup().env("FOUNDRY_DIR", &foundry_dir).args(["--use", "1.0.0"]).assert().success();

    // Attempting to switch to the broken version fails...
    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .args(["--use", "2.0.0"])
        .assert()
        .failure()
        .stderr_eq(str![[r#"
...
[..]failed to run chisel[..]
...
"#]]);

    // ...and every active binary must still point at v1.0.0, not a mix.
    let bin_dir = foundry_dir.join("bin");
    for &bin in BINS {
        let active = bin_dir.join(format!("{bin}{EXE_SUFFIX}"));
        let target = std::fs::read_link(&active)
            .unwrap_or_else(|e| panic!("{bin} is not an active symlink: {e}"));
        assert_eq!(target, good_dir.join(format!("{bin}{EXE_SUFFIX}")), "{bin} was switched");
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

// Empty `FOUNDRYUP_*` env vars are treated as unset, so an empty numeric var
// like `FOUNDRYUP_PR` must not fail clap's integer parsing.
#[test]
fn empty_foundryup_env_is_ignored() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();

    foundryup()
        .env("FOUNDRY_DIR", temp_dir.path().join(".foundry"))
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

// `--list` results are written to stdout.
#[test]
fn list_writes_to_stdout() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");
    let version_dir = foundry_dir.join("versions/foundry-rs/foundry/v1.0.0");
    std::fs::create_dir_all(&version_dir).unwrap();

    for bin in BINS {
        write_fake_bin(&version_dir.join(format!("{bin}{EXE_SUFFIX}")));
    }

    foundryup().env("FOUNDRY_DIR", &foundry_dir).arg("--list").assert().success().stdout_eq(str![
        [r#"
foundryup: foundry-rs/foundry v1.0.0
...
"#]
    ]);
}

// A version directory missing a binary fails the listing.
#[test]
fn list_fails_on_broken_install() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");
    let version_dir = foundry_dir.join("versions/foundry-rs/foundry/v1.0.0");
    std::fs::create_dir_all(&version_dir).unwrap();

    // Install all but the last binary, leaving the version incomplete.
    for bin in &BINS[..BINS.len() - 1] {
        write_fake_bin(&version_dir.join(format!("{bin}{EXE_SUFFIX}")));
    }

    foundryup().env("FOUNDRY_DIR", &foundry_dir).arg("--list").assert().failure().stderr_eq(str![
        [r#"
...
[..]is broken: failed to run [..]
...
"#]
    ]);
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
            write_fake_bin(&bin_path);
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

#[test]
fn migrate_legacy_versions_skips_existing_target() {
    let temp_dir = tempfile::Builder::new().tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");
    let versions_dir = foundry_dir.join("versions");
    let legacy_dir = versions_dir.join("nightly");
    let version_dir = versions_dir.join("foundry-rs/foundry/nightly");

    std::fs::create_dir_all(&legacy_dir).unwrap();
    std::fs::create_dir_all(&version_dir).unwrap();

    for bin in BINS {
        write_fake_bin(&legacy_dir.join(format!("{bin}{EXE_SUFFIX}")));
        write_fake_bin(&version_dir.join(format!("{bin}{EXE_SUFFIX}")));
    }

    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .arg("--list")
        .assert()
        .success()
        .stdout_eq(str![[r#"
foundryup: foundry-rs/foundry nightly
...
"#]])
        .stderr_eq("");

    assert!(legacy_dir.exists());
    assert!(version_dir.exists());
}
