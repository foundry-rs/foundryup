use super::*;

#[test]
fn update_flag_help() {
    foundryup().arg("--help").assert().success().stdout_eq(str![[r#"
...
  -U, --update
          Update foundryup to the latest version
...
"#]]);
}

#[test]
fn update_checks_for_updates() {
    let temp_dir = tempfile::tempdir().unwrap();
    let exe_dir = temp_dir.path().join("exe");
    let foundry_dir = temp_dir.path().join("foundry");
    std::fs::create_dir_all(&exe_dir).unwrap();

    // `-U` self-replaces the running binary. Run a copy (kept separate from
    // FOUNDRY_DIR) so the shared Cargo test binary invoked by every other test
    // via `cargo_bin!` is never clobbered under nextest parallelism.
    let bin = exe_dir.join(format!("foundryup{EXE_SUFFIX}"));
    std::fs::copy(snapbox::cmd::cargo_bin!("foundryup"), &bin).unwrap();

    Command::new(&bin)
        .env("NO_COLOR", "1")
        .env("FOUNDRY_DIR", &foundry_dir)
        .arg("-U")
        .assert()
        .stderr_eq(str![[r#"
...
foundryup: checking for updates...
...
"#]]);
}
