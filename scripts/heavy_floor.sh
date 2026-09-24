#!/usr/bin/env bash
#
# The heavy models, held to floors of their own.
#
# `scripts/library_floor.sh` no longer measures the models named in
# `scripts/heavy_models.txt`: three of them cost seventy-two percent of
# the run half between them, and what they measure is the speed of the
# solver on a large circuit rather than how much of the library this
# compiler reads. The reason for carving them out is in that file.
#
# This is the other half of the arrangement, and it is the half that
# makes it honest. A model taken out of a measurement and given no
# floor has its regression hidden for ever - so these run on a
# schedule, against the same five numbers the main floor holds, over
# exactly the list the main check skips. One file names the set, so the
# two cannot drift apart: what one passes to `--without` the other
# passes to `--only-from`.
#
# Red here is a finding for the next shift rather than a stop signal. A
# benchmark can grow slower because of the machine it was handed, and
# nothing waits on this job - which is also why the times per model are
# not held to a ceiling here: the whole point of these models is that
# they are dear, and a ceiling over three samples on a shared runner
# would fire on the weather.
#
# Usage: scripts/heavy_floor.sh <library directory>
set -euo pipefail

# The counts are small and exact, so they are written as the floors
# they are. Six models stood here with none of them running, and a
# zero floor earned its place by waiting for the day one ran. On
# 2026-09-24 three ReferenceAir moist-air examples joined, and one of
# them, `ReferenceAir.MoistAir`, already runs: it left the main run
# floors and is held here instead, so the run floor stops being zero
# for the first time. Nine models, all nine flatten, one runs; seven of
# them are runnable examples (/tmp/m266/heavy.txt).
FLATTEN_FLOOR=9
RUN_FLOOR=1
RUNNABLE_FLATTEN_FLOOR=7
RUNNABLE_RUN_FLOOR=1

directory="${1:?usage: heavy_floor.sh <library directory>}"
cd "$(dirname "$0")/.."
heavy="scripts/heavy_models.txt"

# The list is read by the check, not by this script, and a list that is
# not there is a failure there rather than a zero here. What this does
# check is that the file names as many models as the floors were set
# over: a name deleted from the list would otherwise lower every count
# and pass, which is the regression this whole file exists to catch.
named="$(grep -cvE '^\s*(#|$)' "$heavy")"
if [ "$named" -ne "$FLATTEN_FLOOR" ]; then
  echo "FLOOR: $heavy names $named model(s), and the floors were set over $FLATTEN_FLOOR"
  echo "       A model added to or taken from that list moves these floors and the main ones."
  exit 1
fi

report="$(./target/release/oxidelica library check --only-from "$heavy" "$directory")"
printf '%s\n' "$report"

flatten_now="$(echo "$report" | sed -n 's/^classes:.*of which \([0-9]*\) flatten.*/\1/p')"
run_now="$(echo "$report" | sed -n 's/^classes:.*flatten and \([0-9]*\) run.*/\1/p')"
runnable_flatten_now="$(echo "$report" | sed -n 's/^runnable.*of which \([0-9]*\) flatten.*/\1/p')"
runnable_run_now="$(echo "$report" | sed -n 's/^runnable.*flatten and \([0-9]*\) run.*/\1/p')"

status=0
short() {
  echo "FLOOR: $1 is $2, and the floor is $3"
  status=1
}
[ "${flatten_now:-0}" -ge "$FLATTEN_FLOOR" ] || short "heavy models flattened" "${flatten_now:-none}" "$FLATTEN_FLOOR"
[ "${run_now:-0}" -ge "$RUN_FLOOR" ] || short "heavy models run" "${run_now:-none}" "$RUN_FLOOR"
[ "${runnable_flatten_now:-0}" -ge "$RUNNABLE_FLATTEN_FLOOR" ] || short "runnable heavy models flattened" "${runnable_flatten_now:-none}" "$RUNNABLE_FLATTEN_FLOOR"
[ "${runnable_run_now:-0}" -ge "$RUNNABLE_RUN_FLOOR" ] || short "runnable heavy models run" "${runnable_run_now:-none}" "$RUNNABLE_RUN_FLOOR"

if [ "$status" -eq 0 ]; then
  echo "OK: of $named heavy models, $flatten_now flatten and $run_now run;"
  echo "OK: runnable $runnable_flatten_now flatten, $runnable_run_now run"
fi
exit "$status"
