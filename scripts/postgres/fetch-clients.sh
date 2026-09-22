#!/usr/bin/env bash
set -euo pipefail

# Refreshes the vendored PostgreSQL client binaries in assets/tools/<arch>/postgresql/.
# Every binary of a given major version MUST come from the same package: pg_dumpall
# refuses to run a pg_dump whose version string differs from its own.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SUITE="bookworm-pgdg"
BASE="https://apt.postgresql.org/pub/repos/apt"
VERSIONS=(12 13 14 15 16 17 18)
ARCHES=(amd64 arm64)

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

for arch in "${ARCHES[@]}"; do
  packages="$work/Packages-$arch"
  curl -fsSL "$BASE/dists/$SUITE/main/binary-$arch/Packages.gz" | gunzip > "$packages"

  for v in "${VERSIONS[@]}"; do
    filename="$(awk -v pkg="postgresql-client-$v" '
      /^Package: /   { current = $2 }
      /^Filename: /  { if (current == pkg) { print $2; exit } }
    ' "$packages")"

    if [ -z "$filename" ]; then
      echo "no postgresql-client-$v for $arch in $SUITE" >&2
      exit 1
    fi

    deb="$work/$arch-$v.deb"
    curl -fsSL -o "$deb" "$BASE/$filename"

    extract="$work/extract-$arch-$v"
    mkdir -p "$extract"
    dpkg-deb -x "$deb" "$extract"

    dest="$REPO_ROOT/assets/tools/$arch/postgresql/postgresql-$v/bin"
    rm -rf "$dest"
    mkdir -p "$dest"
    cp -a "$extract/usr/lib/postgresql/$v/bin/." "$dest/"
    chmod +x "$dest"/*

    echo "$arch postgresql-$v <- $(basename "$filename")"
  done
done
