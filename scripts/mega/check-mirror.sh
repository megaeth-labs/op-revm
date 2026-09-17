#!/usr/bin/env bash
# check-mirror.sh <monorepo-clone> [<mirror-commit>]
#
# Re-checks the snapshot commit against upstream: every file in the commit's
# tree must be byte-identical to rust/op-revm of the monorepo at the revision
# the commit subject names, and Cargo.toml must equal the flattened manifest
# that flatten-manifest.py produces from the same revision. The mirror commit
# defaults to the newest commit whose subject starts with "Mirror op-revm@".
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
clone=$1
commit=${2:-$(git log -1 --format=%H --grep '^Mirror op-revm@')}
subject=$(git log -1 --format=%s "$commit")
short=$(printf '%s' "$subject" | sed -nE 's/^Mirror op-revm@([0-9a-f]+) .*/\1/p')
if [ -z "$short" ]; then
  echo "::error::$commit is not a mirror commit: $subject"
  exit 1
fi
base=$(tr -d '[:space:]' < "$script_dir/base.txt")
case "$base" in "$short"*) ;; *)
  echo "::error::scripts/mega/base.txt ($base) does not match the mirror commit subject ($short)"
  exit 1 ;;
esac
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
full=$("$script_dir/mirror.sh" "$clone" "$base" "$tmp/upstream")
mkdir -p "$tmp/mirror"
git archive "$commit" | tar -x -C "$tmp/mirror"
if diff -r "$tmp/upstream" "$tmp/mirror"; then
  echo "mirror commit ${commit:0:12} equals rust/op-revm at $full (Cargo.toml flattened)"
else
  echo "::error::mirror commit ${commit:0:12} differs from rust/op-revm at $full"
  exit 1
fi
