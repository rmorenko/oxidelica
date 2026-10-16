#!/bin/bash
# Name the models of a list that can raise an event, as the compiler
# counts them: relations whose sign is watched, relations on `time`,
# `sample(...)` clocks and `when` clauses.
#
#   scripts/event_models.sh <list-of-classes> <out.tsv>
#
# Each model is compiled and run to t = 0 only, so the price is the
# flattening. Grepping the example's own text would miss a relation
# inherited from a library block, which is where most of them are.
# Columns: class, status (sources | refused | timeout), then for
# `sources` the four counts - relations, relations on time, samples,
# `when` clauses - and otherwise the reason.
# A model that does not answer within EVENT_CPU seconds (60) is named
# as a timeout, never left out.
set -euo pipefail

list="${1:?usage: event_models.sh <list-of-classes> <out.tsv>}"
out="${2:?usage: event_models.sh <list-of-classes> <out.tsv>}"
[ -s "$list" ] || { echo "event_models.sh: $list is missing or empty" >&2; exit 2; }

here="$(cd "$(dirname "$0")/.." && pwd)"
ox="${OX:-$here/target/release/oxidelica}"
[ -x "$ox" ] || { echo "event_models.sh: no binary at $ox" >&2; exit 2; }
export OXIDELICA_LIB="${OXIDELICA_LIB:-$here/.msl}"
export OX="$ox" EVENT_CPU="${EVENT_CPU:-60}"

probe() {
    local class="$1" log code count
    log=$(mktemp -t evsrc)
    set +e
    (ulimit -t "$EVENT_CPU"; exec "$OX" simulate "$class" --stop 0 -o /dev/null) > "$log" 2>&1
    code=$?
    set -e
    count=$(grep -m1 '^event sources: ' "$log" |
        sed -E 's/^event sources: ([0-9]+) relations, ([0-9]+) on time, ([0-9]+) samples, ([0-9]+) when$/\1\t\2\t\3\t\4/' || true)
    if [ "$code" -eq 0 ] && [ -n "$count" ]; then
        printf '%s\tsources\t%s\n' "$class" "$count"
    elif [ "$code" -eq 152 ] || [ "$code" -eq 137 ] || [ "$code" -eq 158 ]; then
        printf '%s\ttimeout\texit %s after %ss of CPU\n' "$class" "$code" "$EVENT_CPU"
    else
        printf '%s\trefused\texit %s: %s\n' "$class" "$code" \
            "$(grep -v '^note: ' "$log" | tail -1 | cut -c1-200)"
    fi
    rm -f "$log"
}
export -f probe

: > "$out"
grep . "$list" | xargs -P "${EVENT_JOBS:-6}" -n 1 bash -c 'probe "$0"' >> "$out"
echo "event_models.sh: $(wc -l < "$out") lines for $(grep -c . "$list") classes into $out"
