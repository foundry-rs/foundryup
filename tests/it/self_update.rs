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
    let output = foundryup().arg("-U").env("NO_COLOR", "1").output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("checking for updates"),
        "expected 'checking for updates' message, got: {stderr}"
    );
}
