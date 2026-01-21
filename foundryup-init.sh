#!/bin/sh
set -eu

# This script downloads and installs foundryup, the Foundry toolchain manager.
# It detects the platform, downloads the appropriate binary, and runs it.

FOUNDRYUP_REPO="foundry-rs/foundryup"
BASE_DIR="${XDG_CONFIG_HOME:-$HOME}"
FOUNDRY_DIR="${FOUNDRY_DIR:-$BASE_DIR/.foundry}"
FOUNDRYUP_BIN_DIR="$FOUNDRY_DIR/bin"
FOUNDRYUP_IGNORE_VERIFICATION="${FOUNDRYUP_IGNORE_VERIFICATION:-false}"

usage() {
    cat <<EOF
foundryup-init 2.0.0

The installer for foundryup

Usage: foundryup-init.sh [OPTIONS]

Options:
  -v, --verbose   Enable verbose output
  -q, --quiet     Disable progress output
  -y, --yes       Skip confirmation prompt
  -f, --force     Skip attestation verification (INSECURE)
  -h, --help      Print help
  -V, --version   Print version

All other options are passed to foundryup after installation.

Environment variables:
  FOUNDRYUP_VERSION              Install a specific version of foundryup
  FOUNDRYUP_IGNORE_VERIFICATION  Skip attestation verification if set to "true"
EOF
}

main() {
    downloader --check
    need_cmd uname
    need_cmd mktemp
    need_cmd chmod
    need_cmd mkdir
    need_cmd rm
    need_cmd rmdir

    get_architecture || return 1
    local _arch="$RETVAL"
    assert_nz "$_arch" "arch"

    local _passthrough_args=""
    local _need_tty=yes
    local _verbose=no
    local _quiet=no

    for arg in "$@"; do
        case "$arg" in
            -h|--help)
                usage
                exit 0
                ;;
            -V|--version)
                echo "foundryup-init 2.0.0"
                exit 0
                ;;
            -v|--verbose)
                _verbose=yes
                _passthrough_args="$_passthrough_args $arg"
                ;;
            -q|--quiet)
                _quiet=yes
                _passthrough_args="$_passthrough_args $arg"
                ;;
            -f|--force)
                FOUNDRYUP_IGNORE_VERIFICATION=true
                ;;
            -y|--yes)
                _need_tty=no
                _passthrough_args="$_passthrough_args $arg"
                ;;
            *)
                _passthrough_args="$_passthrough_args $arg"
                ;;
        esac
    done

    local _url
    local _attestation_url
    local _base_url
    if [ "${FOUNDRYUP_VERSION+set}" = 'set' ]; then
        say "installing foundryup version $FOUNDRYUP_VERSION"
        _base_url="https://github.com/${FOUNDRYUP_REPO}/releases/download/v${FOUNDRYUP_VERSION}"
        _url="${_base_url}/foundryup_${_arch}"
        _attestation_url="${_base_url}/foundryup_${_arch}.attestation.txt"
    else
        say "installing latest foundryup"
        _base_url="https://github.com/${FOUNDRYUP_REPO}/releases/latest/download"
        _url="${_base_url}/foundryup_${_arch}"
        _attestation_url="${_base_url}/foundryup_${_arch}.attestation.txt"
    fi

    if [ "$_verbose" = "yes" ]; then
        say "url: $_url"
        say "arch: $_arch"
    fi

    local _dir
    if ! _dir="$(ensure mktemp -d)"; then
        exit 1
    fi
    local _file="${_dir}/foundryup"
    local _attestation_file="${_dir}/attestation.txt"
    local _expected_hash=""

    # Download attestation and extract expected hash (unless skipping verification)
    if [ "$FOUNDRYUP_IGNORE_VERIFICATION" = "true" ]; then
        say "skipping attestation verification (--force or FOUNDRYUP_IGNORE_VERIFICATION set)"
    else
        say "downloading attestation..."
        # Use curl/wget directly to avoid the downloader's exit-on-404 behavior
        if try_download "$_attestation_url" "$_attestation_file"; then
            local _attestation_artifact_link
            _attestation_artifact_link="$(head -n1 "$_attestation_file" | tr -d '\r')"

            if [ -n "$_attestation_artifact_link" ] && ! grep -q 'Not Found' "$_attestation_file"; then
                say "verifying attestation..."
                local _sigstore_file="${_dir}/attestation.sigstore.json"

                if try_download "${_attestation_artifact_link}/download" "$_sigstore_file"; then
                    # Extract the payload from the sigstore JSON and decode it
                    local _payload_b64
                    local _payload_json
                    _payload_b64=$(awk '/"payload":/ {gsub(/[",]/, "", $2); print $2; exit}' "$_sigstore_file")
                    _payload_json=$(printf '%s' "$_payload_b64" | base64 -d 2>/dev/null || printf '%s' "$_payload_b64" | base64 -D 2>/dev/null || true)

                    if [ -n "$_payload_json" ]; then
                        # Extract SHA256 hash from the payload
                        _expected_hash=$(printf '%s' "$_payload_json" | grep -oE '"sha256"[[:space:]]*:[[:space:]]*"[a-fA-F0-9]{64}"' | head -1 | grep -oE '[a-fA-F0-9]{64}')
                    fi

                    rm -f "$_sigstore_file"
                fi
            fi

            rm -f "$_attestation_file"
        fi

        if [ -z "$_expected_hash" ]; then
            warn "no attestation found for this release, skipping verification"
        fi
    fi

    say "downloading foundryup..."

    ensure mkdir -p "$_dir"
    ensure downloader "$_url" "$_file" "$_arch"

    # Verify the downloaded binary against the attestation hash
    if [ -n "$_expected_hash" ]; then
        say "verifying binary integrity..."
        local _actual_hash
        _actual_hash=$(compute_sha256 "$_file")

        if [ "$_actual_hash" != "$_expected_hash" ]; then
            err "hash verification failed:
  expected: $_expected_hash
  actual:   $_actual_hash
Use --force to skip verification (INSECURE)"
        fi
        say "binary verified ✓"
    fi

    ensure chmod u+x "$_file"

    if [ ! -x "$_file" ]; then
        err "cannot execute $_file (likely because of mounting /tmp as noexec).
please copy the file to a location where you can execute binaries and run ./foundryup"
    fi

    say "installing foundryup to $FOUNDRYUP_BIN_DIR..."

    ensure mkdir -p "$FOUNDRYUP_BIN_DIR"
    ensure cp "$_file" "$FOUNDRYUP_BIN_DIR/foundryup"
    ensure chmod +x "$FOUNDRYUP_BIN_DIR/foundryup"

    ignore rm "$_file"
    ignore rmdir "$_dir"

    post_install
}

post_install() {
    say ""
    say "foundryup was installed successfully!"
    say ""

    # Check if bin dir is in PATH
    case ":$PATH:" in
        *":$FOUNDRYUP_BIN_DIR:"*)
            say "Run 'foundryup' to install Foundry."
            ;;
        *)
            say "To get started, add foundryup to your PATH:"
            say ""
            say "  export PATH=\"\$PATH:$FOUNDRYUP_BIN_DIR\""
            say ""
            say "Then run 'foundryup' to install Foundry."
            ;;
    esac
}

get_architecture() {
    local _ostype
    local _cputype

    _ostype="$(uname -s)"
    _cputype="$(uname -m)"

    case "$_ostype" in
        Linux)
            if is_musl; then
                _ostype="alpine"
            else
                _ostype="linux"
            fi
            ;;
        Darwin)
            _ostype="darwin"
            ;;
        MINGW* | MSYS* | CYGWIN* | Windows_NT)
            _ostype="win32"
            ;;
        *)
            err "unsupported OS: $_ostype"
            ;;
    esac

    case "$_cputype" in
        x86_64 | x64 | amd64)
            # Check for Rosetta on macOS
            if [ "$_ostype" = "darwin" ] && is_rosetta; then
                _cputype="arm64"
            else
                _cputype="amd64"
            fi
            ;;
        aarch64 | arm64)
            _cputype="arm64"
            ;;
        *)
            err "unsupported architecture: $_cputype"
            ;;
    esac

    RETVAL="${_ostype}_${_cputype}"
}

is_musl() {
    if [ -f /etc/os-release ]; then
        grep -qi "alpine" /etc/os-release 2>/dev/null
        return $?
    fi
    return 1
}

is_rosetta() {
    if [ "$(uname -s)" = "Darwin" ]; then
        if command -v sysctl >/dev/null 2>&1; then
            [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = "1" ]
            return $?
        fi
    fi
    return 1
}

say() {
    printf 'foundryup-init: %s\n' "$1"
}

err() {
    say "$1" >&2
    exit 1
}

warn() {
    say "warning: $1" >&2
}

need_cmd() {
    if ! check_cmd "$1"; then
        err "need '$1' (command not found)"
    fi
}

check_cmd() {
    command -v "$1" > /dev/null 2>&1
}

assert_nz() {
    if [ -z "$1" ]; then
        err "assert_nz $2"
    fi
}

compute_sha256() {
    if check_cmd sha256sum; then
        sha256sum "$1" | cut -d' ' -f1 | sed 's/^\\//'
    elif check_cmd shasum; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        err "need 'sha256sum' or 'shasum' for verification"
    fi
}

# Download without exiting on failure (used for optional files like attestations)
try_download() {
    if check_cmd curl; then
        curl --proto '=https' --tlsv1.2 --silent --fail --location "$1" --output "$2" 2>/dev/null
    elif check_cmd wget; then
        wget --https-only --secure-protocol=TLSv1_2 -q "$1" -O "$2" 2>/dev/null
    else
        return 1
    fi
}

ensure() {
    if ! "$@"; then
        err "command failed: $*"
    fi
}

ignore() {
    "$@"
}

downloader() {
    local _dld
    local _err
    local _status

    if check_cmd curl; then
        _dld=curl
    elif check_cmd wget; then
        _dld=wget
    else
        _dld='curl or wget'
    fi

    if [ "$1" = --check ]; then
        need_cmd "$_dld"
    elif [ "$_dld" = curl ]; then
        _err=$(curl --proto '=https' --tlsv1.2 --silent --show-error --fail --location "$1" --output "$2" 2>&1)
        _status=$?
        if [ -n "$_err" ]; then
            warn "$_err"
            if echo "$_err" | grep -q 404; then
                err "binary for platform '$3' not found, this may be unsupported"
            fi
        fi
        return $_status
    elif [ "$_dld" = wget ]; then
        _err=$(wget --https-only --secure-protocol=TLSv1_2 "$1" -O "$2" 2>&1)
        _status=$?
        if [ -n "$_err" ]; then
            warn "$_err"
            if echo "$_err" | grep -q '404'; then
                err "binary for platform '$3' not found, this may be unsupported"
            fi
        fi
        return $_status
    else
        err "unknown downloader"
    fi
}

main "$@" || exit 1
