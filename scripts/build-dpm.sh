#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_dir="$root/vendor/declarative-postgres-migrate.rs"
expected="a5e868acc0206fa9c3e91b5e36e0b1b111805885"
actual="$(git -C "$source_dir" rev-parse HEAD)"
if [[ "$actual" != "$expected" ]]; then
  echo "production dependency drift: expected $expected, observed $actual" >&2
  exit 1
fi
cargo build --locked --release --manifest-path "$source_dir/Cargo.toml" --bin dpm
printf '%s\n' "$source_dir/target/release/dpm"
