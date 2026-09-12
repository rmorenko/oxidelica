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
# The time per model is held to a ceiling, one for each half. The
# whole check grows longer as more models pass - a model refused early
# used to cost nothing, and the ones that newly pass are the dear
# ones, which is why they were stuck - so the total says little on its
# own and is not held to anything. What says something is the time per
# model that reached each half: that moving is the compiler changing,
# where the total moving alone is coverage changing.
#
# The ceilings are written from the build machine's numbers, which are
# the slower pair: a desk at the time of writing measured 1666ms
# flattening and 889ms running against the machine's 3612-4143 and
# 2449-2657. That is the opposite way round from the counts above,
# which take the lower of the two - and for the same reason. A count
# set from the higher machine goes red on every desk; a time set from
# the faster machine does the same. Both are set where the check can
# be reproduced where it fires.
#
# The headroom is wide on purpose - about two thirds again over the
# dearest run seen, where two runs of the same code on the same
# machine already differ by a seventh. This is not a benchmark and a
# few percent is noise. It is a trap for the kind of regression that
# was paid for once: an index reduction that took the run half from
# 591s to 3153s, a factor of five, went unnoticed for two days and
# then killed two build machine runs on the ninety minute cap. A
# factor of five clears these ceilings by a mile, and a fifth does
# not wake them.
#
# `--slow N` on the check itself is the other half of this pair: the
# ceiling says the compiler got slower, and the list of the dearest
# models by half says where, from the same run rather than from two
# more.
#
# Usage: scripts/library_floor.sh <library directory>
set -euo pipefail

FILES_FLOOR=2671
FLATTEN_FLOOR=820
RUN_FLOOR=459
RUNNABLE_FLATTEN_FLOOR=722
RUNNABLE_RUN_FLOOR=430
# Every file of the library parses. This is a ceiling reached rather
# than a floor to hold, so it is written as the number left over: one
# file that stops parsing takes its whole tree of classes with it, and
# the counts below would hide that behind a handful of models.
UNREAD_CEILING=0
# Milliseconds per model that reached each half. See the note above
# for why these are the build machine's numbers and not a desk's.
FLATTEN_MS_CEILING=7000
RUN_MS_CEILING=4500

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
# The two times per model, taken from the line the check prints. The
# fraction is cut rather than rounded: the shell compares integers,
# and a ceiling wide enough to swallow a seventh of noise does not
# care about a millisecond.
flatten_ms_now="$(echo "$report" | sed -n 's/^time: flattening [0-9.]*s over [0-9]* models (\([0-9]*\)ms each).*/\1/p')"
run_ms_now="$(echo "$report" | sed -n 's/^time:.*running [0-9.]*s over [0-9]* (\([0-9]*\)ms each).*/\1/p')"

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
# The times. A missing number is a failure and not a pass: the line
# the two are read from is printed by the same check that printed the
# counts, so nothing there means the report changed shape, and a
# ceiling that silently stops measuring is the thing this project has
# already been bitten by twice.
over() {
  echo "CEILING: $1 is ${2}ms per model, and the ceiling is ${3}ms"
  status=1
}
if [ -z "${flatten_ms_now:-}" ] || [ -z "${run_ms_now:-}" ]; then
  echo "CEILING: the report did not say what a model cost; the time line changed shape"
  status=1
else
  [ "$flatten_ms_now" -le "$FLATTEN_MS_CEILING" ] || over "flattening" "$flatten_ms_now" "$FLATTEN_MS_CEILING"
  [ "$run_ms_now" -le "$RUN_MS_CEILING" ] || over "running" "$run_ms_now" "$RUN_MS_CEILING"
fi

if [ "$status" -eq 0 ]; then
  echo "OK: $read_now files read, $flatten_now flatten, $run_now run; runnable $runnable_flatten_now flatten, $runnable_run_now run"
  echo "OK: ${flatten_ms_now}ms per model flattening, ${run_ms_now}ms running"
fi
exit "$status"
