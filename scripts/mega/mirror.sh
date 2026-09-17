#!/usr/bin/env bash
# mirror.sh <monorepo-clone> <rev> <output-dir>
#
# Materialise rust/op-revm of the OP monorepo at <rev> into <output-dir> as a
# standalone crate: every file verbatim, Cargo.toml flattened (see
# flatten-manifest.py). A new snapshot commit is made from the result:
#
#   scripts/mega/mirror.sh ~/src/optimism <rev> /tmp/snapshot
#   rsync -a --delete --exclude .git --exclude target /tmp/snapshot/ .   # then restore fork-owned files
#
# The usual flow is documented in MEGAETH-FORK.md ("Moving to a new upstream
# snapshot"); this script only produces the tree.
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
clone=$1
rev=$2
out=$3
prefix=rust/op-revm
full=$(git -C "$clone" rev-parse --verify "$rev^{commit}")
rm -rf "$out"
mkdir -p "$out"
git -C "$clone" archive "$full" "$prefix" | tar -x --strip-components=2 -C "$out"
# cargo metadata needs the whole rust workspace at that revision.
wt=$(mktemp -d)
# A sparse checkout of rust/ is enough for cargo metadata and keeps a partial
# (blobless) clone from fetching the whole monorepo.
git -C "$clone" worktree add -q --detach --no-checkout "$wt" "$full"
git -C "$wt" sparse-checkout set rust
git -C "$wt" checkout -q "$full"
trap 'git -C "$clone" worktree remove --force "$wt"' EXIT
python3 "$script_dir/flatten-manifest.py" "$wt/rust" "$out/Cargo.toml" > "$out/Cargo.toml.flat"
mv "$out/Cargo.toml.flat" "$out/Cargo.toml"
echo "$full"
