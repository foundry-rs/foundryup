//! Tests for the toolchain proxies that `foundryup` links into the bin dir.

use super::*;
use std::path::PathBuf;

/// Installs fake toolchains under `versions/<repo>/<version>` and activates the first one.
fn setup(toolchains: &[&str]) -> (tempfile::TempDir, PathBuf) {
    let temp_dir = tempfile::tempdir().unwrap();
    let foundry_dir = temp_dir.path().join(".foundry");
    for toolchain in toolchains {
        let dir = foundry_dir.join("versions").join(toolchain);
        std::fs::create_dir_all(&dir).unwrap();
        for bin in BINS {
            write_fake_bin(&dir.join(format!("{bin}{EXE_SUFFIX}")));
        }
    }

    let (_, default) = toolchains[0].rsplit_once('/').unwrap();
    foundryup().env("FOUNDRY_DIR", &foundry_dir).args(["--use", default]).assert().success();
    (temp_dir, foundry_dir)
}

#[test]
fn runs_default_toolchain() {
    let (_temp_dir, foundry_dir) =
        setup(&["foundry-rs/foundry/v1.0.0", "foundry-rs/foundry/v2.0.0"]);

    proxy(&foundry_dir, "forge").args(["build", "--sizes"]).assert().success().stdout_eq(str![[
        r#"
forge v1.0.0
args: ["build", "--sizes"]
FOUNDRYUP_TOOLCHAIN: None

"#
    ]]);
}

// The proxy finds its installation from its own path, so `FOUNDRY_DIR` set only when
// installing is not needed afterwards.
#[test]
fn uses_own_installation() {
    let (temp_dir, foundry_dir) = setup(&["foundry-rs/foundry/v1.0.0"]);

    proxy(&foundry_dir, "forge")
        .env("FOUNDRY_DIR", temp_dir.path().join("elsewhere"))
        .arg("-V")
        .assert()
        .success()
        .stdout_eq(str![[r#"
forge v1.0.0

"#]]);
}

#[test]
fn plus_selects_toolchain() {
    let (_temp_dir, foundry_dir) =
        setup(&["foundry-rs/foundry/v1.0.0", "foundry-rs/foundry/v2.0.0"]);

    proxy(&foundry_dir, "cast").args(["+2.0.0", "call", "+x"]).assert().success().stdout_eq(str![
        [r#"
cast v2.0.0
args: ["call", "+x"]
FOUNDRYUP_TOOLCHAIN: Some("2.0.0")

"#]
    ]);
}

#[test]
fn env_selects_toolchain() {
    let (_temp_dir, foundry_dir) =
        setup(&["foundry-rs/foundry/v1.0.0", "foundry-rs/foundry/v2.0.0"]);

    proxy(&foundry_dir, "anvil").env("FOUNDRYUP_TOOLCHAIN", "v2.0.0").assert().success().stdout_eq(
        str![[r#"
anvil v2.0.0
args: []
FOUNDRYUP_TOOLCHAIN: Some("v2.0.0")

"#]],
    );

    // `+toolchain` takes precedence over the environment.
    proxy(&foundry_dir, "anvil")
        .env("FOUNDRYUP_TOOLCHAIN", "v2.0.0")
        .arg("+v1.0.0")
        .assert()
        .success()
        .stdout_eq(str![[r#"
anvil v1.0.0
args: []
FOUNDRYUP_TOOLCHAIN: Some("v1.0.0")

"#]]);

    // An empty variable is ignored.
    proxy(&foundry_dir, "anvil")
        .env("FOUNDRYUP_TOOLCHAIN", "")
        .arg("-V")
        .assert()
        .success()
        .stdout_eq(str![[r#"
anvil v1.0.0

"#]]);
}

// An exported `FOUNDRYUP_TOOLCHAIN` does not change what `foundryup --use` activates or reports.
#[test]
fn env_does_not_affect_activation() {
    let (_temp_dir, foundry_dir) =
        setup(&["foundry-rs/foundry/v1.0.0", "foundry-rs/foundry/v2.0.0"]);

    foundryup()
        .env("FOUNDRY_DIR", &foundry_dir)
        .env("FOUNDRYUP_TOOLCHAIN", "v1.0.0")
        .args(["--use", "v2.0.0"])
        .assert()
        .success()
        .stderr_eq(str![[r#"
foundryup: use - forge v2.0.0 (from forge v1.0.0)
...
"#]]);
    proxy(&foundry_dir, "forge").arg("-V").assert().success().stdout_eq(str![[r#"
forge v2.0.0

"#]]);
}

// Activation links the proxies to the running foundryup. Activating again from the foundryup that
// they link to keeps them, since Windows cannot remove hard links to the running executable.
#[test]
fn reactivation_from_linked_foundryup() {
    let (_temp_dir, foundry_dir) =
        setup(&["foundry-rs/foundry/v1.0.0", "foundry-rs/foundry/v2.0.0"]);
    let installed = foundry_dir.join(format!("bin/foundryup{EXE_SUFFIX}"));
    std::fs::copy(snapbox::cmd::cargo_bin!("foundryup"), &installed).unwrap();

    for version in ["v2.0.0", "v1.0.0"] {
        Command::new(&installed)
            .env("FOUNDRY_DIR", &foundry_dir)
            .args(["--use", version])
            .assert()
            .success();
    }

    proxy(&foundry_dir, "forge").arg("-V").assert().success().stdout_eq(str![[r#"
forge v1.0.0

"#]]);
    #[cfg(unix)]
    for bin in BINS {
        assert_eq!(std::fs::read_link(foundry_dir.join("bin").join(bin)).unwrap(), installed);
    }

    foundryup().env("FOUNDRY_DIR", &foundry_dir).args(["--use", "v2.0.0"]).assert().success();
    proxy(&foundry_dir, "forge").arg("-V").assert().success().stdout_eq(str![[r#"
forge v2.0.0

"#]]);
    #[cfg(unix)]
    for bin in BINS {
        assert_eq!(
            std::fs::read_link(foundry_dir.join("bin").join(bin)).unwrap(),
            snapbox::cmd::cargo_bin!("foundryup")
        );
    }
}

// `stable` and `latest` select the highest installed release by semver, ignoring prereleases.
#[test]
fn stable_selects_newest_release() {
    let (_temp_dir, foundry_dir) = setup(&[
        "foundry-rs/foundry/v1.9.0",
        "foundry-rs/foundry/v1.10.0",
        "foundry-rs/foundry/v2.0.0-rc1",
        "foundry-rs/foundry/nightly-abc",
    ]);

    for channel in ["+stable", "+latest"] {
        proxy(&foundry_dir, "forge").args([channel, "-V"]).assert().success().stdout_eq(str![[
            r#"
forge v1.10.0

"#
        ]]);
    }
}

// `nightly` selects the most recently installed nightly, not the last one by name.
#[cfg(unix)]
#[test]
fn nightly_selects_latest_install() {
    let (_temp_dir, foundry_dir) = setup(&[
        "foundry-rs/foundry/v1.0.0",
        "foundry-rs/foundry/nightly-aaa",
        "foundry-rs/foundry/nightly-bbb",
    ]);
    let older = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::open(foundry_dir.join("versions/foundry-rs/foundry/nightly-bbb"))
        .unwrap()
        .set_modified(older)
        .unwrap();

    proxy(&foundry_dir, "forge").args(["+nightly", "-V"]).assert().success().stdout_eq(str![[r#"
forge nightly-aaa

"#]]);
}

// Other names are looked up like `foundryup --use`: digit-prefixed names that are not version
// numbers stay verbatim, and builds from other repos are found by name.
#[test]
fn names_match_use() {
    let (_temp_dir, foundry_dir) = setup(&[
        "foundry-rs/foundry/v1.0.0",
        "foundry-rs/foundry/123abc",
        "someone/foundry/someone-branch-x",
    ]);

    proxy(&foundry_dir, "chisel").args(["+123abc", "-V"]).assert().success().stdout_eq(str![[r#"
chisel 123abc

"#]]);
    proxy(&foundry_dir, "chisel").args(["+someone-branch-x", "-V"]).assert().success().stdout_eq(
        str![[r#"
chisel someone-branch-x

"#]],
    );
}

#[test]
fn forwards_exit_code() {
    let (_temp_dir, foundry_dir) = setup(&["foundry-rs/foundry/v1.0.0"]);

    proxy(&foundry_dir, "forge").args(["+v1.0.0", "exit", "3"]).assert().code(3);
}

#[test]
fn errors() {
    let (_temp_dir, foundry_dir) = setup(&["foundry-rs/foundry/v1.0.0"]);

    proxy(&foundry_dir, "forge").arg("+1.5.0").assert().failure().stdout_eq("").stderr_eq(str![[
        r#"
foundryup: error: toolchain '1.5.0' is not installed; install it with `foundryup --install 1.5.0`

"#
    ]]);

    proxy(&foundry_dir, "forge").arg("+").assert().failure().stderr_eq(str![[r#"
foundryup: error: missing toolchain name after '+'

"#]]);

    std::fs::remove_file(foundry_dir.join("default-toolchain")).unwrap();
    proxy(&foundry_dir, "forge").assert().failure().stderr_eq(str![[r#"
foundryup: error: no default toolchain is set; run `foundryup` to install one

"#]]);
}

// A toolchain without an optional binary reports it as not installed.
#[test]
fn errors_on_missing_optional_bin() {
    let (_temp_dir, foundry_dir) =
        setup(&["foundry-rs/foundry/v2.0.0", "foundry-rs/foundry/v1.0.0"]);
    let v2 = foundry_dir.join("versions/foundry-rs/foundry/v2.0.0");
    write_fake_bin(&v2.join(format!("solar{EXE_SUFFIX}")));
    foundryup().env("FOUNDRY_DIR", &foundry_dir).args(["--use", "v2.0.0"]).assert().success();

    proxy(&foundry_dir, "solar").arg("+v1.0.0").assert().failure().stderr_eq(str![[r#"
foundryup: error: solar is not installed in toolchain [..]v1.0.0

"#]]);
}
