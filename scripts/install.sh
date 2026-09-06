#!/bin/sh
set -eu

repository='qinodes/stoker'
release_version='__STOKER_RELEASE_VERSION__'
case "$release_version" in
    __STOKER_*)
        release_base_url="https://github.com/$repository/releases/latest/download"
        release_label='latest release'
        ;;
    *)
        release_base_url="https://github.com/$repository/releases/download/v$release_version"
        release_label="v$release_version"
        ;;
esac
install_directory="$HOME/.local/bin"

if [ -z "${HOME:-}" ]; then
    echo 'Could not determine the home directory.' >&2
    exit 1
fi

if ! command -v curl >/dev/null 2>&1; then
    echo 'This installer requires curl.' >&2
    exit 1
fi

if ! command -v tar >/dev/null 2>&1; then
    echo 'This installer requires tar.' >&2
    exit 1
fi

os=$(uname -s)
architecture=$(uname -m)
case "$os:$architecture" in
    Linux:x86_64|Linux:amd64)
        asset='stoker-linux-x86_64.tar.gz'
        ;;
    Darwin:arm64|Darwin:aarch64)
        asset='stoker-macos-arm64.tar.gz'
        ;;
    *)
        echo "Unsupported platform: $os $architecture. Current releases support Linux x86_64 and macOS Apple Silicon." >&2
        exit 1
        ;;
esac

temporary_directory=$(mktemp -d "${TMPDIR:-/tmp}/stoker-install.XXXXXX")
cleanup() {
    rm -rf "$temporary_directory"
}
trap cleanup EXIT HUP INT TERM

archive_path="$temporary_directory/$asset"
checksums_path="$temporary_directory/SHA256SUMS"

echo "Downloading Stoker for $os $architecture ($release_label)..."
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
    "$release_base_url/$asset" --output "$archive_path"
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
    "$release_base_url/SHA256SUMS" --output "$checksums_path"

expected_hash=$(awk -v file="$asset" '$2 == file || $2 == "*" file { print $1; exit }' "$checksums_path")
if [ -z "$expected_hash" ]; then
    echo "Could not find a checksum for $asset." >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    actual_hash=$(sha256sum "$archive_path" | awk '{ print $1 }')
elif command -v shasum >/dev/null 2>&1; then
    actual_hash=$(shasum -a 256 "$archive_path" | awk '{ print $1 }')
else
    echo 'This installer requires sha256sum or shasum for archive verification.' >&2
    exit 1
fi

if [ "$actual_hash" != "$expected_hash" ]; then
    echo 'The downloaded archive failed SHA256 verification.' >&2
    exit 1
fi

tar -xzf "$archive_path" -C "$temporary_directory"
if [ ! -f "$temporary_directory/stoker" ]; then
    echo 'The downloaded archive does not contain the stoker executable.' >&2
    exit 1
fi

mkdir -p "$install_directory"
install -m 0755 "$temporary_directory/stoker" "$install_directory/stoker"

path_line='export PATH="$HOME/.local/bin:$PATH"'
ensure_path_file() {
    path_file=$1
    touch "$path_file"
    if ! grep -Fqx "$path_line" "$path_file"; then
        printf '\n# stoker\n%s\n' "$path_line" >> "$path_file"
    fi
}

ensure_path_file "$HOME/.profile"
case "${SHELL:-}" in
    */bash)
        ensure_path_file "$HOME/.bashrc"
        ;;
    */zsh)
        ensure_path_file "$HOME/.zshrc"
        if [ "$os" = 'Darwin' ]; then
            ensure_path_file "$HOME/.zprofile"
        fi
        ;;
esac

export PATH="$install_directory:$PATH"
echo "Stoker was installed to $install_directory."
echo 'The install directory was added to your user PATH.'
echo 'Open a new terminal, or run: . ~/.profile'
echo 'Try: stoker --version'
