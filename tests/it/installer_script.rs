use std::process::{Command, Stdio};

fn script_without_main() -> String {
    let script = include_str!("../../foundryup-init.sh");
    script.replace("main \"$@\" || exit 1", "")
}

fn run_script_function(function_body: &str) -> std::process::Output {
    let script = script_without_main();
    let full_script = format!("{script}\n\n{function_body}");
    Command::new("sh")
        .arg("-c")
        .arg(&full_script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

#[test]
fn script_has_shebang() {
    let script = include_str!("../../foundryup-init.sh");
    assert!(script.starts_with("#!/bin/sh"), "script should start with shebang");
}

#[test]
fn script_usage_works() {
    let output = run_script_function("usage");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("foundryup-init"), "usage should mention foundryup-init");
    assert!(stdout.contains("--help"), "usage should mention --help");
    assert!(stdout.contains("--version"), "usage should mention --version");
}

#[test]
fn script_help_flag() {
    let output = Command::new("sh")
        .args(["foundryup-init.sh", "--help"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("foundryup-init"));
}

#[test]
fn script_get_architecture_linux_amd64() {
    let script = script_without_main();
    let output = Command::new("sh")
        .args([
            "-c",
            &format!(
                r#"
{script}
uname() {{
    case "$1" in
        -s) echo "Linux" ;;
        -m) echo "x86_64" ;;
    esac
}}
is_musl() {{ return 1; }}
get_architecture
echo "$RETVAL"
"#
            ),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(stdout, "linux_amd64");
}

#[test]
fn script_get_architecture_darwin_arm64() {
    let script = script_without_main();
    let output = Command::new("sh")
        .args([
            "-c",
            &format!(
                r#"
{script}
uname() {{
    case "$1" in
        -s) echo "Darwin" ;;
        -m) echo "arm64" ;;
    esac
}}
get_architecture
echo "$RETVAL"
"#
            ),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(stdout, "darwin_arm64");
}

#[test]
fn script_get_architecture_alpine() {
    let script = script_without_main();
    let output = Command::new("sh")
        .args([
            "-c",
            &format!(
                r#"
{script}
uname() {{
    case "$1" in
        -s) echo "Linux" ;;
        -m) echo "x86_64" ;;
    esac
}}
is_musl() {{ return 0; }}
get_architecture
echo "$RETVAL"
"#
            ),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(stdout, "alpine_amd64");
}

#[test]
fn script_get_architecture_windows() {
    let script = script_without_main();
    let output = Command::new("sh")
        .args([
            "-c",
            &format!(
                r#"
{script}
uname() {{
    case "$1" in
        -s) echo "MINGW64_NT-10.0" ;;
        -m) echo "x86_64" ;;
    esac
}}
get_architecture
echo "$RETVAL"
"#
            ),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(stdout, "win32_amd64");
}

#[test]
fn script_check_cmd_exists() {
    let output = run_script_function("check_cmd sh && echo 'found'");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("found"));
}

#[test]
fn script_check_cmd_not_exists() {
    let output =
        run_script_function("check_cmd nonexistent_cmd_12345 && echo 'found' || echo 'not'");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("not"));
}

#[test]
fn script_need_cmd_fails_for_missing() {
    let output = run_script_function("need_cmd nonexistent_cmd_xyz_12345");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("need 'nonexistent_cmd_xyz_12345'"));
}

#[test]
fn script_assert_nz_success() {
    let output = run_script_function("assert_nz 'value' 'test' && echo 'ok'");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ok"));
}

#[test]
fn script_assert_nz_failure() {
    let output = run_script_function("assert_nz '' 'test'");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("assert_nz test"));
}

#[test]
fn script_say_output() {
    let output = run_script_function("say 'hello world'");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("foundryup-init: hello world"));
}

#[test]
fn script_downloader_check() {
    let output = run_script_function("downloader --check && echo 'ok'");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ok"));
}

#[test]
fn script_ensure_success() {
    let output = run_script_function("ensure true && echo 'ok'");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ok"));
}

#[test]
fn script_ensure_failure() {
    let output = run_script_function("ensure false");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("command failed"));
}

#[test]
fn script_ignore_runs_command() {
    let output = run_script_function("ignore echo 'test'");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("test"));
}

#[test]
fn script_foundryup_repo_defined() {
    let output = run_script_function(r#"echo "$FOUNDRYUP_REPO""#);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("foundry-rs/foundryup"));
}

#[test]
fn script_foundryup_bin_dir_default() {
    let script = script_without_main();
    let output = Command::new("sh")
        .args([
            "-c",
            &format!(
                r#"
unset FOUNDRY_DIR
HOME=/tmp/test_home
{script}
echo "$FOUNDRYUP_BIN_DIR"
"#
            ),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("/tmp/test_home/.foundry/bin"));
}

#[test]
fn script_foundryup_bin_dir_custom() {
    let script = script_without_main();
    let output = Command::new("sh")
        .args([
            "-c",
            &format!(
                r#"
FOUNDRY_DIR=/custom/path
{script}
echo "$FOUNDRYUP_BIN_DIR"
"#
            ),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("/custom/path/bin"));
}
