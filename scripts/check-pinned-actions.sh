#!/usr/bin/env bash
# Fails when a GitHub Actions `uses:` is not pinned to a full commit SHA.
# A tag or branch can be moved to other code; a commit SHA cannot.
# Usage: scripts/check-pinned-actions.sh [workflow-dir]   (default: .github/workflows)
set -euo pipefail
dir="${1:-.github/workflows}"
if [ ! -d "$dir" ]; then
    echo "no such folder: $dir" >&2
    exit 2
fi
status=0
while IFS= read -r hit; do
    file="${hit%%:*}"
    rest="${hit#*:}"
    line="${rest%%:*}"
    text="${rest#*:}"
    # CRLF files, comments (a ref never holds "#") and trailing blanks don't count.
    ref="$(sed -E "s/\r$//; s/^[[:space:]-]*uses:[[:space:]]*//; s/#.*$//; s/[[:space:]]+$//; s/^[\"']//; s/[\"']$//" <<<"$text")"
    case "$ref" in ./*) continue ;; esac # actions in this repository
    if [[ ! "$ref" =~ @[0-9a-f]{40}$ ]]; then
        echo "UNPINNED $file:$line: $ref"
        status=1
    fi
done < <(grep -rnHE '^[[:space:]-]*uses:' --include='*.yml' --include='*.yaml' "$dir" || true)
exit $status
