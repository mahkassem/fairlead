#!/usr/bin/env sh
# Fails when a tracked file names anything on the denylist file or in
# $LEAK_DENYLIST (one pattern per line), which keeps private names untracked.
set -eu
list=".github/leak-denylist.txt"
patterns=$( { grep -v '^#' "$list" || true; printf '%s\n' "${LEAK_DENYLIST:-}"; } \
  | grep -v '^[[:space:]]*$' | paste -sd '|' -)
if [ -z "$patterns" ]; then
  echo "::warning::leak-check: no patterns; set the LEAK_DENYLIST repository variable"
  exit 0
fi
hits=$(git ls-files -z | xargs -0 grep -HniE "$patterns" -- 2>/dev/null | grep -v "^$list:" || true)
if [ -n "$hits" ]; then
  echo "leak-check: private names found:" >&2
  echo "$hits" >&2
  exit 1
fi
echo "leak-check: clean"
