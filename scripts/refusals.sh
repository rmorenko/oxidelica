#!/usr/bin/env bash
# What the library was refused for, gathered into kinds and counted.
#
# Both halves: the models that would not flatten, and the models that
# flattened and then would not run. A barrier is a number from this
# script before a change and a number from it after; a kind that is
# absent prints nothing, which is why the counts are printed as a
# whole list rather than grepped for one name.
#
# Usage: scripts/refusals.sh <library directory> [half]
#   half: refused (default), built, both, or unbalanced
set -euo pipefail

usage="usage: refusals.sh <library directory> [refused|built|both|unbalanced]"
directory="${1:?$usage}"
half="${2:-refused}"
# An argument this script does not know is a failure, not an empty
# report. A pipe that answers a mistyped question with nothing prints
# the same as a pipe that found nothing, and that reading has cost
# this project three rounds of work already.
case "$half" in
refused | built | both | unbalanced) ;;
*)
    echo "refusals.sh: unknown half \`$half'" >&2
    echo "$usage" >&2
    exit 2
    ;;
esac
cd "$(dirname "$0")/.."

report="$(./target/release/oxidelica library check "$directory" --refused)"

kinds() {
    # The message with the model name and the quoted particulars taken
    # out, so that one wording of one barrier counts as one kind. A
    # message that lists the unknowns of an unbalanced model names
    # thousands of them, and the list is cut off: what is being
    # counted is the kind of barrier, not the model behind it.
    sed "s/\`[^\`]*\`/\`X\`/g" |
        sed 's/\[".*/[...]/' |
        sed 's/[0-9][0-9]*/N/g' |
        cut -c1-120 |
        sort | uniq -c | sort -rn
}

# The largest family of all, split along the two axes that are already
# printed: by how far the balance misses and which way, and by the
# chapter the model comes from. A count is taken with the sign because
# too few equations and too many are different illnesses; the chapter
# is there because a hundred models of one chapter behind one figure
# are one shared component rather than a hundred illnesses.
unbalanced() {
    echo "$report" | awk -F'\t' '
        $1 ~ /^  built/ && $2 ~ /unbalanced model/ {
            split($1, a, " ")
            match($2, /[0-9]+ algebraic/); eqs = substr($2, RSTART) + 0
            match($2, /[0-9]+ unknown/);   unk = substr($2, RSTART) + 0
            printf "%+d\t%s\n", eqs - unk, a[2]
        }'
}

if [ "$half" = unbalanced ]; then
    echo "=== how far the balance misses ==="
    unbalanced | cut -f1 | sort -n | uniq -c
    echo "=== how far it misses, by chapter ==="
    unbalanced | awk -F'[\t.]' '{ print $1, $3 }' | sort | uniq -c | sort -rn
fi
if [ "$half" = refused ] || [ "$half" = both ]; then
    echo "=== would not flatten ==="
    echo "$report" | sed -n 's/^  refused  [^	]*	//p' | kinds
fi
if [ "$half" = built ] || [ "$half" = both ]; then
    echo "=== flattened, would not run ==="
    echo "$report" | sed -n 's/^  built    [^	]*	//p' | kinds
fi
