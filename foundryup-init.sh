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

set -u

FOUNDRYUP_REPO="foundry-rs/foundryup"
FOUNDRYUP_BIN_DIR="${FOUNDRY_DIR:-$HOME/.foundry}/bin"

usage() {
    cat <<EOF
foundryup-init 2.0.0

The installer for foundryup

Usage: foundryup-init.sh [OPTIONS]

Options:
  -v, --verbose   Enable verbose output
  -q, --quiet     Disable progress output
  -y              Skip confirmation prompt
  -h, --help      Print help
  -V, --version   Print version

All other options are passed to foundryup after installation.

Environment variables:
  FOUNDRYUP_VERSION   Install a specific version of foundryup
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
            -y)
                _need_tty=no
                _passthrough_args="$_passthrough_args $arg"
                ;;
            *)
                _passthrough_args="$_passthrough_args $arg"
                ;;
        esac
    done

    local _url
    if [ "${FOUNDRYUP_VERSION+set}" = 'set' ]; then
        say "installing foundryup version $FOUNDRYUP_VERSION"
        _url="https://github.com/${FOUNDRYUP_REPO}/releases/download/v${FOUNDRYUP_VERSION}/foundryup_${_arch}"
    else
        say "installing latest foundryup"
        _url="https://github.com/${FOUNDRYUP_REPO}/releases/latest/download/foundryup_${_arch}"
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

    say "downloading foundryup..."

    ensure mkdir -p "$_dir"
    ensure downloader "$_url" "$_file" "$_arch"
    ensure chmod u+x "$_file"

    if [ ! -x "$_file" ]; then
        err "cannot execute $_file (likely because of mounting /tmp as noexec)."
        err "please copy the file to a location where you can execute binaries and run ./foundryup"
        exit 1
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
            exit 1
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
            exit 1
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
