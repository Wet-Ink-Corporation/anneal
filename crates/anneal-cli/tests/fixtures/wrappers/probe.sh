#!/usr/bin/env bash
# Runs every row of probes.txt against the wrappers installed here and prints the table with
# what each actually ran (see README.md). Each word W1..W9 of a row is replaced by a recorder
# that writes its own name and exits, so a row never runs anything but recorders, `true` and
# the wrapper itself; nothing it names is deleted or changed.
#
#   probe.sh            print the table as observed here
#   probe.sh --check    exit 1 when an observation differs from probes.txt
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
table="$here/probes.txt"
work=$(mktemp -d)
trap 'rm -r -- "$work"' EXIT

# Fresh recorders for each row: a wrapper given one as a file (`script -q W1`) overwrites it.
recorders() {
    mkdir -p "$work/bin"
    local n
    for n in 1 2 3 4 5 6 7 8 9; do
        printf '#!/bin/sh\necho W%s > "%s/ran"\n' "$n" "$work" > "$work/bin/W$n"
        chmod +x "$work/bin/W$n"
    done
}

observe() {
    local command=$1
    command=$(sed -E "s#W([1-9])#$work/bin/W\\1#g" <<<"$command")
    : > "$work/ran"
    recorders
    (cd "$work" && printf 'x\n' | TERM=dumb timeout 3 bash -c "$command") >/dev/null 2>&1 || true
    if [ -s "$work/ran" ]; then cat "$work/ran"; else echo -; fi
}

versions() {
    printf '# sudo %s; %s; %s; %s; %s\n' \
        "$(sudo --version | sed -n '1s/^Sudo version //p')" \
        "$(env --version | sed -n '1s/^env (GNU \(.*\)) \(.*\)$/GNU \1 \2/p')" \
        "$(xargs --version | sed -n '1s/^xargs (GNU \(.*\)) \(.*\)$/GNU \1 \2/p')" \
        "$(watch -v | sed -n '1s/^watch from //p')" \
        "$(script --version | sed -n '1s/^script from //p')"
}

render() {
    local line expected command
    while IFS= read -r line; do
        case $line in
            '# sudo '*) versions ;;
            '#'* | '') printf '%s\n' "$line" ;;
            *)
                IFS=$'\t' read -r expected command <<<"$line"
                printf '%s\t%s\n' "$(observe "$command")" "$command"
                ;;
        esac
    done < "$table"
}

if [ "${1:-}" = --check ]; then
    diff -u "$table" <(render)
else
    render
fi
