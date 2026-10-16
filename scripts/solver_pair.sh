#!/bin/bash
# Run each named model of the libraries to its own stop time on two
# solvers and print, one line per model and solver, what the run ended
# in: the count of events that changed something and the final point.
#
#   scripts/solver_pair.sh <list-of-classes> <out.tsv> [side-a] [side-b]
#
# A side is a solver (`dopri45`, `rk4`) or a solver at a tolerance that
# overrides the model's own (`dopri45@1e-8`): the second form holds one
# solver against itself, which catches what depends on the size of the
# step where two solvers are not both able to run the model.
#
# A model that runs and prints a wrong number passes the floors, the
# census and the diff of run lists: it runs, it does not refuse, it does
# not drop out of the list. Two solvers agreeing on a model is the
# witness that it does not lie, and `scripts/solver_pair.py` reads this
# file to say which models they agree on.
#
# The class is named to `simulate` directly rather than through a
# wrapper `model W extends M; end W;`, which loses M's experiment
# annotation: the stop time comes from the model itself and from
# nowhere else. A run that outlasts its CPU ceiling (PAIR_CPU seconds,
# 60 by default) is written down as `timeout` by name, never as an
# empty line, because an empty line would read as "no difference".
#
# Columns: class, solver, status (ran | timeout | refused), wall
# wall seconds, events, final point or the refusal's last line.
set -euo pipefail

list="${1:?usage: solver_pair.sh <list-of-classes> <out.tsv> [solver-a] [solver-b]}"
out="${2:?usage: solver_pair.sh <list-of-classes> <out.tsv> [solver-a] [solver-b]}"
solver_a="${3:-dopri45}"
solver_b="${4:-rk4}"
[ -s "$list" ] || { echo "solver_pair.sh: $list is missing or empty" >&2; exit 2; }

here="$(cd "$(dirname "$0")/.." && pwd)"
ox="${OX:-$here/target/release/oxidelica}"
[ -x "$ox" ] || { echo "solver_pair.sh: no binary at $ox" >&2; exit 2; }
usage=$("$ox" 2>&1 || true)
case "$usage" in *"oxidelica simulate"*) ;; *)
    echo "solver_pair.sh: $ox does not answer; a binary copied over another is killed at start" >&2
    exit 2
    ;;
esac
export OXIDELICA_LIB="${OXIDELICA_LIB:-$here/.msl}"
# The witnesses - small models where a lost crossing shows - are named
# beside the library classes as `Witnesses.*`.
export MODELICAPATH="$here/scripts/solver_pair_witnesses${MODELICAPATH:+:$MODELICAPATH}"
export OX="$ox" PAIR_CPU="${PAIR_CPU:-60}"
jobs="${PAIR_JOBS:-6}"

one() {
    local class="$1" solver="$2" scratch log code started ended status events last
    scratch=$(mktemp -t pair)
    log=$(mktemp -t pairlog)
    started=$(date +%s)
    set +e
    # A side is a solver, or a solver at a tolerance - `dopri45@1e-8` -
    # so that one solver can be held against itself. The side keeps its
    # whole name in the output, because the reader sorts the lines of a
    # model into sides by that column, and two sides named `dopri45`
    # would be one side.
    local name="${solver%%@*}" flags=()
    case "$solver" in *@*) flags=(--tolerance "${solver#*@}") ;; esac
    (ulimit -t "$PAIR_CPU"; exec "$OX" simulate "$class" --solver "$name" ${flags[@]+"${flags[@]}"} -o /dev/null) \
        > "$log" 2>&1
    code=$?
    set -e
    ended=$(date +%s)
    events=$(grep -m1 '^events: ' "$log" | sed 's/^events: //' || true)
    if [ "$code" -eq 0 ] && [ -n "$events" ]; then
        status=ran
        last=$(grep -m1 '^final point: ' "$log" | cut -c1-20000 || true)
    elif [ "$code" -eq 152 ] || [ "$code" -eq 137 ] || [ "$code" -eq 158 ]; then
        # SIGXCPU is the ceiling; SIGKILL follows it when the soft limit
        # is ignored. Either is the ceiling and not the model's refusal.
        status=timeout
        events=-
        last="exit $code after ${PAIR_CPU}s of CPU"
    else
        status=refused
        events=-
        last="exit $code: $(grep -v '^note: ' "$log" | tail -1 | cut -c1-300)"
    fi
    rm -f "$scratch" "$log"
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$class" "$solver" "$status" \
        "$((ended - started))" "$events" "$last"
}
export -f one

: > "$out"
while read -r class; do
    [ -n "$class" ] || continue
    printf '%s %s\n' "$class" "$solver_a"
    # `-` as the second solver runs the first alone: the reference is
    # checked by one run, the pair is needed only to admit to it.
    [ "$solver_b" = "-" ] || printf '%s %s\n' "$class" "$solver_b"
done < "$list" | xargs -P "$jobs" -n 2 bash -c 'one "$0" "$1"' >> "$out"

lines=$(wc -l < "$out")
echo "solver_pair.sh: $lines lines for $(grep -c . "$list") classes into $out"
