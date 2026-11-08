#!/usr/bin/env bash
# Fails when the documents drift from the code in ways a script can see.
#   bash scripts/check-docs.sh [repo-root]
#
# 1. Every capability string in `local_capabilities()` is documented in docs/WIRE_PROTOCOL.md.
# 2. docs/WIRE_PROTOCOL.md does not call itself a draft.
# 3. docs/THREAT_MODEL.md has a row for each of the security-relevant things 0.3.0 adds.
# 4. Every ticked row in docs/v2/PROGRESS.md names a milestone id (M<n>.<n>) that appears in a commit subject,
#    so a row cannot be ticked for work that was never committed.
set -uo pipefail
root="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$root" || exit 2
status=0
fail() { echo "DRIFT: $*"; status=1; }

# 1. capabilities
proto=crates/pb-protocol/src/lib.rs
wire=docs/WIRE_PROTOCOL.md
if [ ! -f "$proto" ] || [ ! -f "$wire" ]; then echo "missing $proto or $wire" >&2; exit 2; fi
caps="$(awk '/pub fn local_capabilities/,/^}/' "$proto" | grep -oE 'CAP_[A-Z0-9_]+' | sort -u)"
[ -n "$caps" ] || fail "found no capability in local_capabilities()"
for c in $caps; do
    value="$(grep -E "pub const $c: &str" "$proto" | sed -E 's/.*= "([^"]+)".*/\1/')"
    [ -n "$value" ] || { fail "$c has no value in $proto"; continue; }
    grep -q "\`$value\`" "$wire" || fail "capability \`$value\` ($c) is not documented in $wire"
done

# 2. status words
head -3 "$wire" | grep -qi 'draft' && fail "$wire still calls itself a draft"

# 3. threat model rows
tm=docs/THREAT_MODEL.md
for needle in "Disconnect everything" "PBD2" "update" "clipboard history" "partial" "firewall"; do
    grep -qi -- "$needle" "$tm" || fail "$tm has no row for: $needle"
done

# 4. ticked rows have a commit
progress=docs/v2/PROGRESS.md
if git rev-parse --git-dir >/dev/null 2>&1 && [ -f "$progress" ]; then
    # A subject may name a range ("M4.5-M4.8") or the plans of several milestones ("plans for M9 to M11");
    # both are expanded to the ids they cover.
    subjects="$(git log --format=%s | awk '
        { print }
        match($0, /M[0-9]+\.[0-9]+-M[0-9]+\.[0-9]+/) {
            r = substr($0, RSTART, RLENGTH); split(r, a, "-")
            split(substr(a[1], 2), x, "."); split(substr(a[2], 2), y, ".")
            if (x[1] == y[1]) for (i = x[2]; i <= y[2]; i++) print "M" x[1] "." i
        }
        match($0, /plans for M[0-9]+ to M[0-9]+/) {
            r = substr($0, RSTART, RLENGTH); n = split(r, w, " ")
            lo = substr(w[3], 2) + 0; hi = substr(w[5], 2) + 0
            for (m = lo; m <= hi; m++) print "M" m ".0"
        }')"
    for id in $(grep -E '^- \[x\] M[0-9]+\.[0-9]+[a-z]?' "$progress" | sed -E 's/^- \[x\] (M[0-9]+\.[0-9]+[a-z]?).*/\1/' | sort -u); do
        grep -qE "(^|[^0-9A-Za-z.])$id([^0-9A-Za-z]|$)" <<<"$subjects" || fail "$progress ticks $id but no commit subject names it"
    done
fi
[ "$status" -eq 0 ] && echo "docs match the code"
exit "$status"
