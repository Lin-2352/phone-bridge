#!/usr/bin/env bash
# Tests for scripts/check-pinned-actions.sh. Usage: bash scripts/tests/check-pinned-actions.test.sh
set -uo pipefail
check="$(cd "$(dirname "$0")/.." && pwd)/check-pinned-actions.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
sha=0123456789abcdef0123456789abcdef01234567
fails=0
expect() { # name, want exit code, dir
    bash "$check" "$3" > "$tmp/out" 2>&1
    local got=$?
    if [ "$got" -ne "$2" ]; then
        echo "FAIL $1: exit $got, want $2"; sed 's/^/    /' "$tmp/out"; fails=$((fails + 1))
    else
        echo "ok   $1"
    fi
}
case_dir() { mkdir -p "$tmp/$1"; printf '%b' "$2" > "$tmp/$1/w.yml"; echo "$tmp/$1"; }

expect "pinned passes" 0 "$(case_dir pinned "jobs:\n  a:\n    steps:\n      - uses: o/r@$sha # v1\n")"
expect "tag fails" 1 "$(case_dir tag "jobs:\n  a:\n    steps:\n      - uses: o/r@v1\n")"
expect "missing folder fails" 2 "$tmp/no-such-folder"
expect "CRLF pinned passes" 0 "$(case_dir crlf "jobs:\r\n  a:\r\n    steps:\r\n      - uses: o/r@$sha # v1\r\n")"
expect "CRLF without comment passes" 0 "$(case_dir crlf2 "      - uses: o/r@$sha\r\n")"
expect "trailing spaces pass" 0 "$(case_dir trailing "      - uses: o/r@$sha   \n")"
expect "comment without space passes" 0 "$(case_dir hash "      - uses: o/r@$sha#v1\n")"
expect "local action passes" 0 "$(case_dir local "      - uses: ./.github/actions/x\n")"
[ "$fails" -eq 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
