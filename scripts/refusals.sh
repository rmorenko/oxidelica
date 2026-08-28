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
#   half: refused (default), built, or both
set -euo pipefail

directory="${1:?usage: refusals.sh <library directory> [refused|stalled|both]}"
half="${2:-refused}"
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

if [ "$half" = refused ] || [ "$half" = both ]; then
    echo "=== would not flatten ==="
    echo "$report" | sed -n 's/^  refused  [^	]*	//p' | kinds
fi
if [ "$half" = built ] || [ "$half" = both ]; then
    echo "=== flattened, would not run ==="
    echo "$report" | sed -n 's/^  built    [^	]*	//p' | kinds
fi
