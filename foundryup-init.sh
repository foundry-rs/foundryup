#!/bin/sh
# shellcheck shell=dash
# shellcheck disable=SC2039  # local is non-POSIX

# This script downloads and installs foundryup, the Foundry toolchain manager.
# It detects the platform, downloads the appropriate binary, and runs it.

# It runs on Unix shells like {a,ba,da,k,z}sh. It uses the common `local`
# extension. Note: Most shells limit `local` to 1 var per line, contra bash.

# Some versions of ksh have no `local` keyword. Alias it to `typeset`, but
# beware this makes variables global with f()-style function syntax in ksh93.
has_local() {
    # shellcheck disable=SC2034  # deliberately unused
    local _has_local
}

has_local 2>/dev/null || alias local=typeset

set -eu

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
  FOUNDRYUP_MAX_RETRIES          Retries after a failed download (default: 5)
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
    local _ext
    _ext=$(get_ext "$_arch")
    if [ "${FOUNDRYUP_VERSION+set}" = 'set' ]; then
        say "installing foundryup version $FOUNDRYUP_VERSION"
        _base_url="https://github.com/${FOUNDRYUP_REPO}/releases/download/v${FOUNDRYUP_VERSION}"
        _url="${_base_url}/foundryup_${_arch}${_ext}"
        _attestation_url="${_base_url}/foundryup_${_arch}.attestation.txt"
    else
        say "installing latest foundryup"
        _base_url="https://github.com/${FOUNDRYUP_REPO}/releases/latest/download"
        _url="${_base_url}/foundryup_${_arch}${_ext}"
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
        if download_optional "$_attestation_url" "$_attestation_file"; then
            local _attestation_artifact_link
            _attestation_artifact_link="$(head -n1 "$_attestation_file" | tr -d '\r')"

            if [ -n "$_attestation_artifact_link" ] && ! grep -q 'Not Found' "$_attestation_file"; then
                say "verifying attestation..."
                local _sigstore_file="${_dir}/attestation.sigstore.json"

                if download_optional "${_attestation_artifact_link}/download" "$_sigstore_file"; then
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

get_ext() {
    case "$1" in
        win32_*) echo ".exe" ;;
        *) echo "" ;;
    esac
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

# Shared transport for binary and attestation downloads. Keep the retry count,
# HTTP statuses and 1, 2, 4, 8, 16 second backoff in sync with src/retry.rs.
# RETVAL is the final HTTP status, used by callers to distinguish a missing file.
try_download() {
    local _dld
    if check_cmd curl; then
        _dld=curl
    elif check_cmd wget; then
        _dld=wget
    else
        err "need 'curl' or 'wget'"
    fi

    local _max_retries
    _max_retries=$(awk -v value="${FOUNDRYUP_MAX_RETRIES:-}" 'BEGIN {
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", value)
        if (value ~ /^[+]?[0-9]+$/ && value + 0 <= 4294967295) printf "%.0f\n", value
        else print 5
    }')
    local _attempt=0
    local _delay=1
    local _status
    local _http_status
    local _err
    local _retryable
    local _host="${1#https://}"
    _host="${_host%%/*}"
    _host="${_host%%:*}"
    _host="${_host%.}"

    while :; do
        # Overwrite the destination on every attempt; never append partial bytes.
        if [ "$_dld" = curl ]; then
            if _http_status=$(curl --proto '=https' --tlsv1.2 --silent --show-error --fail --location \
                --write-out '%{http_code}' "$1" --output "$2"); then
                RETVAL="$_http_status"
                return 0
            else
                _status=$?
            fi
        else
            # Disable wget's own retry loop so both transports have one budget.
            if _err=$(LC_ALL=C wget --https-only --secure-protocol=TLSv1_2 --server-response --tries=1 \
                "$1" -O "$2" 2>&1); then
                RETVAL=200
                return 0
            else
                _status=$?
            fi
            _http_status=$(printf '%s\n' "$_err" | awk '$1 ~ /^HTTP\// { code=$2 } END { print code }')
            warn "$_err"
        fi
        RETVAL="$_http_status"
        _retryable=false
        case "$_dld:$_status" in
            curl:22|wget:8)
                case "$_http_status" in
                    403|408|429|500|502|503|504) _retryable=true ;;
                esac
                ;;
            # curl setup/local I/O errors and permanent HTTP errors are not transient.
            curl:1|curl:2|curl:3|curl:4|curl:23|curl:26|curl:27) ;;
            curl:*|wget:4|wget:5) _retryable=true ;;
        esac
        case "$_host" in
            github.com|*.github.com|githubusercontent.com|*.githubusercontent.com) ;;
            *) _retryable=false ;;
        esac
        if [ "$_retryable" = false ] || [ "$_attempt" -ge "$_max_retries" ]; then
            return "$_status"
        fi
        _attempt=$((_attempt + 1))
        warn "download failed; retrying in ${_delay}s (${_attempt}/${_max_retries})"
        sleep "$_delay"
        if [ "$_delay" -lt 16 ]; then _delay=$((_delay * 2)); fi
    done
}

# Only a genuine 404 is optional. Exhausted transient errors must not skip verification.
download_optional() {
    if try_download "$1" "$2"; then
        return 0
    fi
    if [ "$RETVAL" = 404 ]; then return 1; fi
    err "failed to download $1"
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
    local _status

    if [ "$1" = --check ]; then
        if ! check_cmd curl && ! check_cmd wget; then err "need 'curl' or 'wget'"; fi
        return 0
    fi
    if try_download "$1" "$2"; then
        return 0
    else
        _status=$?
    fi
    if [ "$RETVAL" = 404 ]; then
        err "binary for platform '$3' not found, this may be unsupported"
    fi
    return "$_status"
}

main "$@" || exit 1
