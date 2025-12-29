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
    foundryup().arg("-U").assert().stderr_eq(str![[r#"
...
foundryup: checking for updates...
...
"#]]);
}
