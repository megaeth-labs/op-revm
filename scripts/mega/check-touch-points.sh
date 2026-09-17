#!/usr/bin/env bash
# check-touch-points.sh BASE HEAD
#
# Every upstream file a change modifies or deletes must have a row in the
# "Upstream touch points" table of MEGAETH-FORK.md, so that the next snapshot
# knows what to re-apply. Files the fork owns (workflows, scripts/mega, the
# fork documents, the megaeth module, the toolchain and cargo config) are not
# upstream files and are skipped; files the change adds are new, not touch
# points. Only the table section is searched for the row.
set -euo pipefail
base=$1
head=$2
doc=MEGAETH-FORK.md
table=$(awk '/^## Upstream touch points/{f=1; next} /^## /{f=0} f' "$doc")
fail=0
while IFS= read -r file; do
  case "$file" in
    .github/*|scripts/mega/*|.cargo/*|MEGAETH-FORK.md|REVIEW.md|PROVENANCE.md|rust-toolchain.toml|deny.toml|Cargo.lock|.gitignore|rustfmt.toml|src/megaeth.rs) continue ;;
  esac
  if ! grep -Fq "\`$file\`" <<< "$table"; then
    echo "::error file=$file::modified upstream file has no row in the touch-point table of $doc"
    fail=1
  fi
done < <(git diff --name-only --diff-filter=MD "$base...$head")
if [ "$fail" -eq 0 ]; then
  echo "every modified upstream file has a touch-point row"
fi
exit "$fail"
