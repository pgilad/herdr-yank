#!/usr/bin/env sh
set -eu

REPO_OWNER="pgilad"
REPO_NAME="herdr-yank"
BIN_DIR="bin"
BIN_PATH="$BIN_DIR/herdr-yank"

# Temporary download directory; the EXIT trap removes it, also after a signal.
tmpdir=""
cleanup() {
    if [ -n "$tmpdir" ]; then
        rm -rf "$tmpdir"
        tmpdir=""
    fi
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

log() {
    printf '%s\n' "$*" >&2
}

plugin_version() {
    awk -F'"' '/^version = "/ { print $2; exit }' herdr-plugin.toml
}

archive_name() {
    version="$1"
    target="$2"
    printf '%s-v%s-%s.tar.gz' "$REPO_NAME" "$version" "$target"
}

release_url() {
    version="$1"
    archive="$2"
    printf 'https://github.com/%s/%s/releases/download/v%s/%s' "$REPO_OWNER" "$REPO_NAME" "$version" "$archive"
}

# Prints the release target of this machine. Fails when no prebuilt binary exists for it, so
# the caller can build from source instead.
current_target() {
    case "$(uname -s)/$(uname -m)" in
        Darwin/arm64 | Darwin/aarch64) printf 'aarch64-apple-darwin' ;;
        Linux/x86_64 | Linux/amd64) printf 'x86_64-unknown-linux-musl' ;;
        *) return 1 ;;
    esac
}

have_command() {
    command -v "$1" >/dev/null 2>&1
}

download_to() {
    url="$1"
    destination="$2"

    if have_command curl; then
        curl -fsSL "$url" -o "$destination"
    elif have_command wget; then
        wget -qO "$destination" "$url"
    else
        log "Need curl or wget to install release binary"
        return 1
    fi
}

# Prints the sha256 of a file as a bare hex digest.
sha256_of() {
    if have_command shasum; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    elif have_command sha256sum; then
        sha256sum "$1" | awk '{ print $1 }'
    else
        return 1
    fi
}

# Fails closed: a missing checksum file or checksum tool aborts the download path so the
# caller falls back to building from source instead of running an unverified binary.
verify_archive() {
    archive_path="$1"
    checksum_path="$2"

    if [ ! -s "$checksum_path" ]; then
        log "Release checksum was not published; refusing the prebuilt binary"
        return 1
    fi

    expected=$(awk '{ print $1; exit }' "$checksum_path")
    if [ -z "$expected" ]; then
        log "Release checksum file is malformed; refusing the prebuilt binary"
        return 1
    fi

    if ! actual=$(sha256_of "$archive_path"); then
        log "Need shasum or sha256sum to verify the release archive"
        return 1
    fi

    if [ "$expected" != "$actual" ]; then
        log "Checksum mismatch for $(basename "$archive_path")"
        log "  expected $expected"
        log "  actual   $actual"
        return 1
    fi

    log "Verified sha256 $actual"
}

# Replaces the installed binary with one rename. A failed step leaves the old binary in
# place, and a running copy of the old binary does not block the update.
install_binary() {
    source_path="$1"
    staged="$BIN_PATH.tmp.$$"

    mkdir -p "$BIN_DIR" || return 1
    if cp "$source_path" "$staged" && chmod +x "$staged" && mv -f "$staged" "$BIN_PATH"; then
        return 0
    fi
    rm -f "$staged"
    log "Could not install $BIN_PATH"
    return 1
}

# Every step checks its own status, because `set -e` has no effect in a function that an
# `if` condition calls.
download_release_binary() {
    version="$1"
    target="$2"
    archive=$(archive_name "$version" "$target")
    url=$(release_url "$version" "$archive")
    tmpdir=$(mktemp -d) || return 1

    log "Downloading $url"
    download_to "$url" "$tmpdir/$archive" || return 1
    download_to "$url.sha256" "$tmpdir/$archive.sha256" || return 1
    verify_archive "$tmpdir/$archive" "$tmpdir/$archive.sha256" || return 1
    if ! tar -xzf "$tmpdir/$archive" -C "$tmpdir"; then
        log "Could not extract $archive"
        return 1
    fi
    if [ ! -f "$tmpdir/herdr-yank" ]; then
        log "$archive does not contain herdr-yank"
        return 1
    fi
    install_binary "$tmpdir/herdr-yank" || return 1
    cleanup
}

# Builds into ./target with an explicit target directory, so a global CARGO_TARGET_DIR does
# not move the binary.
build_from_source() {
    log "Building from local source with cargo"
    cargo build --release --locked --target-dir target
    install_binary target/release/herdr-yank
}

main() {
    version=$(plugin_version)

    if [ "${HERDR_YANK_BUILD_FROM_SOURCE:-0}" = "1" ]; then
        if ! have_command cargo; then
            log "HERDR_YANK_BUILD_FROM_SOURCE=1 requires cargo"
            exit 1
        fi

        build_from_source
        log "Installed $BIN_PATH from local source"
        exit 0
    fi

    if target=$(current_target); then
        if download_release_binary "$version" "$target"; then
            log "Installed $BIN_PATH for $target"
            exit 0
        fi
        cleanup
        log "No usable prebuilt binary for version $version on $target"
    else
        log "No prebuilt binary exists for $(uname -s) $(uname -m)"
    fi

    if ! have_command cargo; then
        log "Cargo is unavailable, thus Herdr Yank cannot be built from source"
        exit 1
    fi

    build_from_source
    log "Installed $BIN_PATH from local source"
}

main "$@"
