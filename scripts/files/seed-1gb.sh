#!/usr/bin/env bash
set -euo pipefail

SIZE_MB=${SIZE_MB:-1024}
ROOT=${ROOT:-./test-files}
TARGETS=${TARGETS:-archive snapshot sync copy}

bytes() { echo $(( SIZE_MB * $1 / 100 * 1024 * 1024 )); }

random_file() {
  mkdir -p "$(dirname "$1")"
  head -c "$2" /dev/urandom > "$1"
}

random_tree() {
  mkdir -p "$1"
  head -c "$2" /dev/urandom | split -b "$3" -a 4 -d - "$1/part-"
}

text_file() {
  mkdir -p "$(dirname "$1")"
  head -c "$2" < <(base64 -w 76 /dev/urandom) > "$1"
}

log_file() {
  mkdir -p "$(dirname "$1")"
  head -c "$2" < <(yes "2026-10-18T12:00:00Z INFO GET /api/backups 200 12ms") > "$1"
}

seed() {
  local dir=$1
  rm -rf "$dir"/{media,projects,docs,logs,cache,build,scratch} "$dir/latest.bin"
  mkdir -p "$dir"

  for i in 1 2 3 4 5 6; do
    random_file "$dir/media/video-$i.bin" "$(bytes 10)"
  done
  for i in $(seq -w 1 20); do
    random_tree "$dir/projects/p$i/src" "$(bytes 1)" 128K
  done
  for i in 1 2 3 4; do
    text_file "$dir/docs/report-$i.txt" "$(bytes 2)"
  done
  log_file "$dir/logs/app.log" "$(bytes 4)"
  random_tree "$dir/projects/p01/node_modules/pkg" "$(bytes 4)" 32K
  random_tree "$dir/cache" "$(bytes 2)" 1M
  random_tree "$dir/build" "$(bytes 1)" 1M
  random_file "$dir/scratch/session.tmp" "$(bytes 1)"
  ln -s media/video-1.bin "$dir/latest.bin"

  echo "$dir: $(du -sh "$dir" | cut -f1), $(find "$dir" -type f | wc -l) files"
}

for target in $TARGETS; do
  seed "${ROOT:?}/$target"
done
