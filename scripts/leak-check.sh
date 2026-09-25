#!/usr/bin/env sh
# Fails when a tracked file names anything on the denylist.
set -eu
list=".github/leak-denylist.txt"
patterns=$(grep -v '^#' "$list" | grep -v '^[[:space:]]*$' | paste -sd '|' -)
hits=$(git ls-files -z | xargs -0 grep -niE "$patterns" -- 2>/dev/null | grep -v "^$list:" || true)
if [ -n "$hits" ]; then
  echo "leak-check: private names found:" >&2
  echo "$hits" >&2
  exit 1
fi
echo "leak-check: clean"
