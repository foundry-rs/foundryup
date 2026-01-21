use std::{
    io::Write,
    process::{Command, Stdio},
};

fn normalize_line_endings(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "")
}

fn script_without_main() -> String {
    let script = include_str!("../../foundryup-init.sh");
    normalize_line_endings(script).replace("main \"$@\" || exit 1", "")
}

fn run_script(script: &str) -> std::process::Output {
    let normalized = normalize_line_endings(script);
    let mut temp_file = tempfile::NamedTempFile::new().unwrap();
    temp_file.write_all(normalized.as_bytes()).unwrap();
    temp_file.flush().unwrap();

    Command::new("sh")
        .arg(temp_file.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

fn run_script_function(function_body: &str) -> std::process::Output {
    let script = script_without_main();
    let full_script = format!("{script}\n\n{function_body}");
    run_script(&full_script)
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
    let output = run_script(&format!(
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
    ));
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(stdout, "linux_amd64");
}

#[test]
fn script_get_architecture_darwin_arm64() {
    let script = script_without_main();
    let output = run_script(&format!(
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
    ));
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(stdout, "darwin_arm64");
}

#[test]
fn script_get_architecture_alpine() {
    let script = script_without_main();
    let output = run_script(&format!(
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
    ));
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(stdout, "alpine_amd64");
}

#[test]
fn script_get_architecture_windows() {
    let script = script_without_main();
    let output = run_script(&format!(
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
    ));
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected success, got {:?}\nstdout: {}\nstderr: {}",
        output.status,
        stdout,
        stderr
    );
    assert!(stdout.contains("ok"), "stdout missing 'ok': {}", stdout);
}

#[test]
fn script_assert_nz_failure() {
    let output = run_script_function("assert_nz '' 'test'");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "expected failure, got {:?}\nstdout: {}\nstderr: {}",
        output.status,
        stdout,
        stderr
    );
    assert!(stderr.contains("assert_nz test"), "stderr missing 'assert_nz test': {}", stderr);
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
    let output = run_script(&format!(
        r#"
unset FOUNDRY_DIR
unset XDG_CONFIG_HOME
HOME=/tmp/test_home
{script}
echo "$FOUNDRYUP_BIN_DIR"
"#
    ));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("/tmp/test_home/.foundry/bin"));
}

#[test]
fn script_foundryup_bin_dir_custom() {
    let script = script_without_main();
    let output = run_script(&format!(
        r#"
FOUNDRY_DIR=/custom/path
{script}
echo "$FOUNDRYUP_BIN_DIR"
"#
    ));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("/custom/path/bin"));
}

#[test]
fn script_downloads_foundryup() {
    let temp_dir = std::env::temp_dir().join(format!("foundryup-test-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let bin_dir = temp_dir.join("bin");
    let foundryup_path = bin_dir.join("foundryup");

    let output = Command::new("sh")
        .args(["foundryup-init.sh", "-y"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("FOUNDRY_DIR", &temp_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "script failed:\nstdout: {stdout}\nstderr: {stderr}");
    assert!(
        foundryup_path.exists(),
        "foundryup binary not found at {foundryup_path:?}\nstdout: {stdout}\nstderr: {stderr}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&foundryup_path).unwrap();
        let permissions = metadata.permissions();
        assert!(permissions.mode() & 0o111 != 0, "foundryup should be executable");
    }

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn script_compute_sha256_known_value() {
    let output = run_script_function(
        r#"
echo -n "hello" > /tmp/sha_test_file
hash=$(compute_sha256 /tmp/sha_test_file)
rm -f /tmp/sha_test_file
echo "$hash"
"#,
    );
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(
        stdout, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        "compute_sha256 should match known SHA256 of 'hello'"
    );
}

#[test]
fn script_compute_sha256_format() {
    let output = run_script_function(
        r#"
echo -n "test" > /tmp/sha_format_test
hash=$(compute_sha256 /tmp/sha_format_test)
rm -f /tmp/sha_format_test

# Check length is 64 and format is hex
if [ "${#hash}" -eq 64 ] && printf '%s' "$hash" | grep -qE '^[a-fA-F0-9]{64}$'; then
    echo "format_ok"
else
    echo "format_bad: $hash"
fi
"#,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("format_ok"), "SHA256 should be 64 hex characters");
}

#[test]
fn script_try_download_success() {
    let output = run_script_function(
        r#"
if try_download "https://github.com" /tmp/try_download_test; then
    echo "download_ok"
    rm -f /tmp/try_download_test
else
    echo "download_failed"
fi
"#,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("download_ok"), "try_download should succeed for valid URL");
}

#[test]
fn script_try_download_failure_no_exit() {
    let output = run_script_function(
        r#"
if try_download "https://github.com/nonexistent-404-page-xyz" /tmp/try_download_fail; then
    echo "unexpected_success"
else
    echo "graceful_failure"
fi
"#,
    );
    assert!(output.status.success(), "script should not exit on try_download failure");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("graceful_failure"),
        "try_download should return false without exiting"
    );
}

#[test]
fn script_force_flag_documented() {
    let output = Command::new("sh")
        .args(["foundryup-init.sh", "--help"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("-f, --force") && stdout.contains("attestation"),
        "help should document --force flag for skipping attestation"
    );
}

#[test]
fn script_downloads_with_attestation_verification() {
    let temp_dir =
        std::env::temp_dir().join(format!("foundryup-attest-test-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let bin_dir = temp_dir.join("bin");
    let foundryup_path = bin_dir.join("foundryup");

    let output = Command::new("sh")
        .args(["foundryup-init.sh", "-y"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("FOUNDRY_DIR", &temp_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "script failed:\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(foundryup_path.exists(), "foundryup binary should be installed");

    let combined = format!("{stdout}{stderr}");
    let attestation_attempted = combined.contains("downloading attestation")
        || combined.contains("verifying attestation")
        || combined.contains("binary verified")
        || combined.contains("no attestation found");
    assert!(
        attestation_attempted,
        "attestation verification should be attempted. Output: {combined}"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn script_downloads_with_force_skips_attestation() {
    let temp_dir =
        std::env::temp_dir().join(format!("foundryup-force-test-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let output = Command::new("sh")
        .args(["foundryup-init.sh", "-y", "--force"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("FOUNDRY_DIR", &temp_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "script failed:\nstdout: {stdout}\nstderr: {stderr}"
    );

    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("skipping attestation verification"),
        "--force should skip attestation. Output: {combined}"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}
