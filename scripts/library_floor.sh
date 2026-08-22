#!/usr/bin/env bash
#
# How much of the standard library this compiler reads, held to a floor.
#
# The numbers this project reports - how many files parse, how many
# example models flatten, how many run - were measured by hand before
# every commit and believed on trust afterwards. Nothing stopped one of
# them going down. This is what stops it.
#
# A floor rather than an exact number, for two reasons. Going up is the
# point, and should not need the threshold edited in the same commit.
# And two models sit on a numerical edge - an algebraic loop that
# converges in fifty Newton iterations or does not - so the run count
# is one of a pair from one process to the next. The floor is set below
# the lower of the two.
#
# Usage: scripts/library_floor.sh <library directory>
set -euo pipefail

FILES_FLOOR=2649
FLATTEN_FLOOR=388
RUN_FLOOR=37

directory="${1:?usage: library_floor.sh <library directory>}"
cd "$(dirname "$0")/.."

report="$(./target/release/oxidelica library check "$directory")"
echo "$report" | head -1
echo "$report" | grep '^classes:'

read_now="$(echo "$report" | sed -n 's/^files: \([0-9]*\) read.*/\1/p')"
flatten_now="$(echo "$report" | sed -n 's/.*of which \([0-9]*\) flatten.*/\1/p')"
run_now="$(echo "$report" | sed -n 's/.*flatten and \([0-9]*\) run.*/\1/p')"

status=0
short() {
  echo "FLOOR: $1 is $2, and the floor is $3"
  status=1
}
[ "${read_now:-0}" -ge "$FILES_FLOOR" ] || short "files read" "${read_now:-none}" "$FILES_FLOOR"
[ "${flatten_now:-0}" -ge "$FLATTEN_FLOOR" ] || short "models flattened" "${flatten_now:-none}" "$FLATTEN_FLOOR"
[ "${run_now:-0}" -ge "$RUN_FLOOR" ] || short "models run" "${run_now:-none}" "$RUN_FLOOR"

if [ "$status" -eq 0 ]; then
  echo "OK: $read_now files read, $flatten_now flatten, $run_now run"
fi
exit "$status"
