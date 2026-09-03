#!/usr/bin/env bash
#
# How much of the standard library this compiler reads, held to a floor.
#
# The numbers this project reports - how many files parse, how many
# example models flatten, how many run - were measured by hand before
# every commit and believed on trust afterwards. Nothing stopped one of
# them going down. This is what stops it.
#
# A floor rather than an exact number: going up is the point, and
# should not need the threshold edited in the same commit.
#
# The run halves differ by a model between the build machine and a
# desk - 342 there against 341 here at the time of writing - so the
# floors are set from the LOWER of the two. Set from the higher, the
# build machine would stay green while every desk went red, which is
# the worst way round for a check nobody can reproduce. The names are
# printed above for exactly this: the difference is one model, and
# subtracting the two lists is how it gets found.
#
# A full measurement is dear - it reads every model of the library -
# so a run of small fixes may be measured once at the end rather than
# after each. What the run may not do is end without measuring: the
# commit that raises the floors names the commits it covers, and the
# numbers it writes are numbers this script printed, not numbers
# anybody expected. A floor raised on an expectation is the same trust
# this script was written to replace.
#
# The time is reported but not held to anything. The whole check grows
# longer as more models pass - a model refused early used to cost
# nothing, and the ones that newly pass are the dear ones, which is
# why they were stuck - so the total says little on its own. What says
# something is the time per model that reached each half: that moving
# is the compiler changing, where the total moving alone is coverage
# changing. Reading them side by side is what tells one from the other.
#
# Usage: scripts/library_floor.sh <library directory>
set -euo pipefail

FILES_FLOOR=2671
FLATTEN_FLOOR=817
RUN_FLOOR=341
RUNNABLE_FLATTEN_FLOOR=719
RUNNABLE_RUN_FLOOR=338
# Every file of the library parses. This is a ceiling reached rather
# than a floor to hold, so it is written as the number left over: one
# file that stops parsing takes its whole tree of classes with it, and
# the counts below would hide that behind a handful of models.
UNREAD_CEILING=0

directory="${1:?usage: library_floor.sh <library directory>}"
cd "$(dirname "$0")/.."

# The names as well as the counts: the run half differs between one
# machine and another, and a difference nobody can name is a
# difference nobody can fix. The list goes to a file rather than the
# log, and the log gets the run half of it, which is where the
# machines disagree.
report="$(./target/release/oxidelica library check --list "$directory")"
ran_list="$(echo "$report" | grep '^  ran   ' | sed 's/^  ran   //' | sort)"
printf '%s\n' "$ran_list" > /tmp/oxidelica_ran.txt
echo "models that ran: $(wc -l < /tmp/oxidelica_ran.txt)"
sed 's/^/  ran   /' /tmp/oxidelica_ran.txt
report="$(echo "$report" | grep -v '^  \(flat\|ran\)  ')"
# A measuring pipe does not cut its own output short. `... | head -1`
# has `head` close the pipe on the line it wanted, `echo` takes a
# SIGPIPE for writing into a closed one, and `pipefail` turns that
# into the whole script failing - a green measurement reported as a
# red job. It hid on a desk, where the report fits a pipe buffer and
# `echo` finishes before `head` leaves, and fired every time on the
# build machine, where the list of models that ran does not fit.
#
# The rule this is the third instance of: a measuring pipe must not
# lie about its result. It does not swallow stderr, it does not
# answer nothing where nothing ran, and it does not cut its own
# output short. Where one line is wanted, take it without a pipe.
printf '%s\n' "${report%%$'\n'*}"
echo "$report" | grep -E '^(classes:|runnable examples|time:)'

read_now="$(echo "$report" | sed -n 's/^files: \([0-9]*\) read.*/\1/p')"
unread_now="$(echo "$report" | sed -n 's/^files: [0-9]* read, \([0-9]*\) not read.*/\1/p')"
flatten_now="$(echo "$report" | sed -n 's/^classes:.*of which \([0-9]*\) flatten.*/\1/p')"
run_now="$(echo "$report" | sed -n 's/^classes:.*flatten and \([0-9]*\) run.*/\1/p')"
runnable_flatten_now="$(echo "$report" | sed -n 's/^runnable.*of which \([0-9]*\) flatten.*/\1/p')"
runnable_run_now="$(echo "$report" | sed -n 's/^runnable.*flatten and \([0-9]*\) run.*/\1/p')"

status=0
short() {
  echo "FLOOR: $1 is $2, and the floor is $3"
  status=1
}
[ "${read_now:-0}" -ge "$FILES_FLOOR" ] || short "files read" "${read_now:-none}" "$FILES_FLOOR"
[ "${unread_now:-1}" -le "$UNREAD_CEILING" ] || {
  echo "FLOOR: ${unread_now:-some} file(s) did not parse, and the ceiling is $UNREAD_CEILING"
  status=1
}
[ "${flatten_now:-0}" -ge "$FLATTEN_FLOOR" ] || short "models flattened" "${flatten_now:-none}" "$FLATTEN_FLOOR"
[ "${run_now:-0}" -ge "$RUN_FLOOR" ] || short "models run" "${run_now:-none}" "$RUN_FLOOR"
[ "${runnable_flatten_now:-0}" -ge "$RUNNABLE_FLATTEN_FLOOR" ] || short "runnable models flattened" "${runnable_flatten_now:-none}" "$RUNNABLE_FLATTEN_FLOOR"
[ "${runnable_run_now:-0}" -ge "$RUNNABLE_RUN_FLOOR" ] || short "runnable models run" "${runnable_run_now:-none}" "$RUNNABLE_RUN_FLOOR"

if [ "$status" -eq 0 ]; then
  echo "OK: $read_now files read, $flatten_now flatten, $run_now run; runnable $runnable_flatten_now flatten, $runnable_run_now run"
fi
exit "$status"
