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
# Every refusal already prints both the counts and the names the
# matching could not pair, so how many names were named can be held
# against how far the balance misses without the compiler being
# touched. The two readings are different illnesses and want
# different work. Named exactly as many as the deficit, and the model
# is honestly short of that many equations - the barrier is whatever
# does not write them. Named more than the deficit, and the equations
# may well be there: the matching saw the names and could not pair
# them, and the barrier is in what it can reach through.
#
# The names are the six the message prints plus the tail it counts,
# `and N more`, which is why both are read.
unbalanced() {
    echo "$report" | awk -F'\t' '
        $1 ~ /^  built/ && $2 ~ /unbalanced model/ {
            split($1, a, " ")
            match($2, /[0-9]+ algebraic/); eqs = substr($2, RSTART) + 0
            match($2, /[0-9]+ unknown/);   unk = substr($2, RSTART) + 0
            # What the refusal named. The list follows the last
            # semicolon and the kind that introduces it, its entries
            # separated by commas, with any remainder counted off.
            tail = $2
            sub(/^.*(nothing determines|nothing is left for) /, "", tail)
            more = 0
            if (match(tail, / and [0-9]+ more$/)) {
                more = substr(tail, RSTART + 5) + 0
                tail = substr(tail, 1, RSTART - 1)
            }
            named = more
            if (tail != "") { named += split(tail, _ignored, ", ") }
            deficit = eqs - unk; if (deficit < 0) deficit = -deficit
            reading = (named > deficit) ? "matching-fell-short" : "honestly-short"
            printf "%+d\t%s\t%d\t%d\t%s\n", eqs - unk, a[2], named, deficit, reading
        }'
}

if [ "$half" = unbalanced ]; then
    echo "=== how far the balance misses ==="
    unbalanced | cut -f1 | sort -n | uniq -c
    echo "=== how far it misses, by chapter ==="
    unbalanced | awk -F'[\t.]' '{ print $1, $3 }' | sort | uniq -c | sort -rn
    echo "=== named against deficit ==="
    unbalanced | cut -f5 | sort | uniq -c | sort -rn
    echo "=== the matching fell short, by chapter ==="
    unbalanced | awk -F'\t' '$5 == "matching-fell-short" { print $2 }' |
        awk -F. '{ print $1 "." $2 }' | sort | uniq -c | sort -rn
    echo "=== honestly short, by chapter ==="
    unbalanced | awk -F'\t' '$5 == "honestly-short" { print $2 }' |
        awk -F. '{ print $1 "." $2 }' | sort | uniq -c | sort -rn
fi
if [ "$half" = refused ] || [ "$half" = both ]; then
    echo "=== would not flatten ==="
    echo "$report" | sed -n 's/^  refused  [^	]*	//p' | kinds
fi
if [ "$half" = built ] || [ "$half" = both ]; then
    echo "=== flattened, would not run ==="
    echo "$report" | sed -n 's/^  built    [^	]*	//p' | kinds
fi
