#!/usr/bin/env sh
set -eu

project_directory=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_directory"

version=$(awk '
    /^\[package\][[:space:]]*$/ {
        in_package = 1
        package_name = 0
    }
    in_package && /^[[]/ && !/^\[package\][[:space:]]*$/ {
        in_package = 0
    }
    in_package && /^name[[:space:]]*=[[:space:]]*"stoker-engine"[[:space:]]*$/ {
        package_name = 1
    }
    in_package && package_name && /^version[[:space:]]*=/ {
        value = $0
        sub(/^[^\"]*\"/, "", value)
        sub(/\".*$/, "", value)
        version = value
        found++
    }
    END {
        if (found != 1) exit 1
        print version
    }
' Cargo.toml) || {
    echo "Could not read the stoker-engine version from Cargo.toml." >&2
    exit 1
}

if ! printf '%s\n' "$version" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'; then
    echo "Invalid stoker-engine version '$version' in Cargo.toml." >&2
    exit 1
fi

printf '%s\n' "$version"
