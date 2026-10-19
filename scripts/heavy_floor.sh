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
# Cut in two on Roman's word of 2026-10-19, after the scheduled job was
# cut at its 240 minute ceiling with 21 of 23 models read. The cut is
# the library job's of 2026-10-13, made with the same hand: two passes
# on two runners and a judge that adds their two reports up and holds
# the SUM to the floors below. The floors are not divided between the
# passes - two sets of floors would be two ledgers, and a regression in
# one basket would hide behind headroom in the other.
#
#   --pass a  the list without the names in `scripts/heavy_second.txt`
#   --pass b  the names in `scripts/heavy_second.txt` alone
#   --judge   the two reports, added up, against the floors
#
# A pass holds nothing: a half is not judged against floors that stand
# for the whole. The judge first asks the binary which models each pass
# reads and refuses, by name, a model of the list read by neither pass
# or by both - otherwise a name that fell out of both files would be
# judged by nobody while every floor still held.
#
# Usage: scripts/heavy_floor.sh <library directory>
#        scripts/heavy_floor.sh --pass a|b <library directory> <report>
#        scripts/heavy_floor.sh --judge <library directory> <a report> <b report>
set -euo pipefail

# The counts are small and exact, so they are written as the floors
# they are. Six models stood here with none of them running, and a
# zero floor earned its place by waiting for the day one ran. On
# 2026-09-24 three ReferenceAir moist-air examples joined, and one of
# them, `ReferenceAir.MoistAir`, already runs: it left the main run
# floors and is held here instead, so the run floor stops being zero
# for the first time. Nine models, all nine flatten, one runs; seven of
# them are runnable examples (/tmp/m266/heavy.txt).
#
# On 2026-09-30 `Spice3.Examples.Oscillator` joined, a runnable example
# that runs: 233s and 456922 Newton steps alone. It left the main
# flatten floors and is held here:
#
#   flatten          10 = 9 + 1
#   run               2 = 1 + 1
#   runnable flatten  8 = 7 + 1
#   runnable run      2 = 1 + 1
#
# Measured from one binary built from e45a2aa (/tmp/m318/heavy.txt).
#
# On 2026-10-06 five refused giants joined on Roman's word, the
# dearest of the library job by the quiet desk: DynamicPipesWith-
# TraceSubstances, BranchingPipes12, R134a2, DynamicPipesAndFittings
# and R134a1. All five flatten, none runs, all five are runnable
# examples (/tmp/m365/five_only.txt), so they left the main flatten
# floors and are held here:
#
#   flatten          15 = 10 + 5
#   run               2 =  2 + 0
#   runnable flatten 13 =  8 + 5
#   runnable run      2 =  2 + 0
#
# On 2026-10-12 eight refused giants joined on Roman's word, the
# refused eight of the ten dearest to run by the quiet desk
# (/tmp/m404/slow.txt): IMC_YDarc, LineForceWithTwoMasses, Fourbar2,
# Engine1b, SeriesPipes13, DynamicPipeInitialization, SeriesPipes12 and
# BranchingPipes18. `--only-from` the eight, one binary built from
# ffd3bcb (/tmp/m405/eight_only.txt), printed 8 / 0 and runnable 8 / 0
# in 255s of flattening and 2737s of running, so:
#
#   flatten          23 = 15 + 8
#   run               2 =  2 + 0
#   runnable flatten 21 = 13 + 8
#   runnable run      2 =  2 + 0
#
# These are the eight's own print added to the floors that stood, not
# a print of the whole list; the scheduled job's next run is the first
# over all twenty-three, and it is the one these floors answer to.
# That job last took 98 minutes and these add about 48 of a runner's,
# inside its 180.
FLATTEN_FLOOR=23
RUN_FLOOR=2
RUNNABLE_FLATTEN_FLOOR=21
RUNNABLE_RUN_FLOOR=2

# A path given before the `cd` below, made absolute so it still names
# the same file after it.
absolute() { echo "$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"; }

mode=whole
case "${1:-}" in
  --pass)
    mode=pass
    pass="${2:?usage: heavy_floor.sh --pass a|b <library directory> <report>}"
    directory="${3:?usage: heavy_floor.sh --pass a|b <library directory> <report>}"
    pass_report="$(absolute "${4:?usage: heavy_floor.sh --pass a|b <library directory> <report>}")"
    ;;
  --judge)
    mode=judge
    directory="${2:?usage: heavy_floor.sh --judge <library directory> <a report> <b report>}"
    a_report="$(absolute "${3:?usage: heavy_floor.sh --judge <library directory> <a report> <b report>}")"
    b_report="$(absolute "${4:?usage: heavy_floor.sh --judge <library directory> <a report> <b report>}")"
    ;;
  *) directory="${1:?usage: heavy_floor.sh <library directory>}" ;;
esac
cd "$(dirname "$0")/.."
heavy="scripts/heavy_models.txt"
# The second basket. Overridable only so the judge can be seen red and
# green on a stand; CI names no other file.
second="${HEAVY_SECOND:-scripts/heavy_second.txt}"

# What each pass passes to the check, in one place, so that the pass
# and the judge's question about its set cannot drift apart.
pass_flags() {
  case "$1" in
    a) echo "--only-from $heavy --without $second" ;;
    b) echo "--only-from $second" ;;
    *)
      echo "usage: the heavy passes are a and b, not '$1'" >&2
      return 1
      ;;
  esac
}

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

# One pass: the check over its basket, written whole to a file for the
# judge, and its own counts printed for the eye. Nothing is held here.
if [ "$mode" = pass ]; then
  flags="$(pass_flags "$pass")"
  # shellcheck disable=SC2086
  ./target/release/oxidelica library check $flags "$directory" > "$pass_report"
  grep -E '^(files:|classes:|runnable examples|time:)' "$pass_report"
  exit 0
fi

if [ "$mode" = judge ]; then
  # The guard, before any count is believed: the two baskets cover the
  # list between them and hold no model twice, asked of the binary with
  # the filter the check itself applies.
  sets="$(mktemp -d)"
  trap 'rm -rf "$sets"' EXIT
  names_of() {
    # shellcheck disable=SC2068
    OXIDELICA_QUIET=1 ./target/release/oxidelica library check --models $@ "$directory" |
      sed -n 's/^  model  //p'
  }
  names_of --only-from "$heavy" > "$sets/all"
  # shellcheck disable=SC2046
  names_of $(pass_flags a) > "$sets/a"
  # shellcheck disable=SC2046
  names_of $(pass_flags b) > "$sets/b"
  ./scripts/library_floor.sh --partition-check "$sets/all" "$sets/a" "$sets/b"
  # A name in the second basket that the list does not hold is a model
  # meant to be judged in pass b and judged nowhere.
  stray="$(grep -v '^#' "$second" | sed '/^[[:space:]]*$/d' | sort -u | comm -23 - "$sets/all" || true)"
  if [ -n "$stray" ]; then
    echo "$stray" | sed 's/^/PARTITION: the second basket names a model outside the heavy list: /'
    exit 1
  fi
  # And each report is the report of its basket.
  for side in a b; do
    file="$a_report"
    [ "$side" = b ] && file="$b_report"
    read_models="$(sed -n 's/^classes:.*example models: \([0-9]*\),.*/\1/p' "$file")"
    want="$(wc -l < "$sets/$side" | tr -d ' ')"
    if [ "${read_models:-none}" != "$want" ]; then
      echo "PARTITION: the pass $side report read ${read_models:-no} models, and its basket holds $want"
      exit 1
    fi
  done
  # The two reports added up into the shape of one. Each count is
  # summed by its place on its line, and the times are summed as
  # seconds for the eye; nothing here holds a time.
  report="$(awk '
    function nums(s, out,   n) {
      split("", out); n = 0
      while (match(s, /[0-9]+(\.[0-9]+)?/)) { out[++n] = substr(s, RSTART, RLENGTH) + 0; s = substr(s, RSTART + RLENGTH) }
      return n
    }
    FNR == 1 { f++ }
    /^files:/ { nf[f]++; if (f == 1) files = $0 }
    /^classes:/ { nc[f]++; nums($0, t); C = t[1]; M += t[2]; F += t[3]; R += t[4] }
    /^runnable examples/ { nr[f]++; nums($0, t); rb_n += t[1]; rb_f += t[2]; rb_r += t[3] }
    /^time:/ { nt[f]++; nums($0, t); fl_s += t[1]; fl_n += t[2]; ru_s += t[4]; ru_n += t[5] }
    END {
      for (g = 1; g <= 2; g++)
        if (nf[g] != 1 || nc[g] != 1 || nr[g] != 1 || nt[g] != 1) {
          print "JUDGE: report " g " does not have one line of each count; it is not a pass report" > "/dev/stderr"
          exit 1
        }
      if (f != 2) { print "JUDGE: two reports were wanted" > "/dev/stderr"; exit 1 }
      print files
      printf "classes: %d; example models: %d, of which %d flatten and %d run\n", C, M, F, R
      printf "runnable examples (experiment or Example icon): %d, of which %d flatten and %d run\n", rb_n, rb_f, rb_r
      printf "time: flattening %.0fs over %d models; running %.0fs over %d, the two passes added\n", fl_s, fl_n, ru_s, ru_n
    }' "$a_report" "$b_report")"
else
  report="$(./target/release/oxidelica library check --only-from "$heavy" "$directory")"
fi
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
