#!/usr/bin/env sh
set -eu

if [ "$#" -ne 1 ]; then
    echo "Usage: make version VERSION=1.2.2" >&2
    exit 1
fi

version=$1
if ! printf '%s\n' "$version" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'; then
    echo "Invalid version '$version'. Expected a semantic version such as 1.2.2." >&2
    exit 1
fi

project_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$project_directory"

toml_tmp=$(mktemp "${TMPDIR:-/tmp}/stoker-Cargo.toml.XXXXXX")
lock_tmp=$(mktemp "${TMPDIR:-/tmp}/stoker-Cargo.lock.XXXXXX")
cleanup() {
    rm -f "$toml_tmp" "$lock_tmp"
}
trap cleanup EXIT HUP INT TERM

awk -v version="$version" '
    /^\[package\][[:space:]]*$/ {
        in_package = 1
        package_name = ""
    }
    in_package && /^[[]/ && !/^\[package\][[:space:]]*$/ {
        in_package = 0
    }
    in_package && /^name[[:space:]]*=[[:space:]]*"stoker-engine"[[:space:]]*$/ {
        package_name = "stoker-engine"
    }
    in_package && package_name == "stoker-engine" && /^version[[:space:]]*=/ {
        sub(/"[^"]*"/, "\"" version "\"")
        updated++
    }
    { print }
    END {
        if (updated != 1) exit 1
    }
' Cargo.toml > "$toml_tmp" || {
    echo "Could not find exactly one stoker-engine package version in Cargo.toml." >&2
    exit 1
}

awk -v version="$version" '
    /^\[\[package\]\][[:space:]]*$/ {
        in_package = 1
        target = 0
    }
    in_package && /^[[]/ && !/^\[\[package\]\][[:space:]]*$/ {
        in_package = 0
        target = 0
    }
    in_package && /^name[[:space:]]*=[[:space:]]*"stoker-engine"[[:space:]]*$/ {
        target = 1
    }
    in_package && target && /^version[[:space:]]*=/ {
        sub(/"[^"]*"/, "\"" version "\"")
        updated++
        target = 0
    }
    { print }
    END {
        if (updated != 1) exit 1
    }
' Cargo.lock > "$lock_tmp" || {
    echo "Could not find exactly one stoker-engine package version in Cargo.lock." >&2
    exit 1
}

mv "$toml_tmp" Cargo.toml
mv "$lock_tmp" Cargo.lock
trap - EXIT HUP INT TERM
echo "Updated stoker-engine version to $version in Cargo.toml and Cargo.lock."
