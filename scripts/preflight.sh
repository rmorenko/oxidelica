#!/usr/bin/env bash
#
# Everything CI checks, before the push rather than after it.
#
# CI runs six jobs, and it is easy to run the two that are on the mind
# at the time - the tests and the library floor - push, and find out
# from the machine that a linter or the coverage threshold had
# something to say. That round trip costs several minutes and a red
# commit on the branch. This runs the same checks here.
#
# What is skipped and why: the three-platform matrix, since this is one
# machine; the advisories job, which asks about the world rather than
# about the commit and is on a weekly schedule of its own. Everything
# else is the same command CI runs, so a pass here means a pass there
# unless the difference is the platform itself.
#
# Usage: scripts/preflight.sh [--quick] [--heavy] [<library directory>]
#
#   --quick  the fast checks only - formatting, clippy, tests, examples.
#            Coverage and the linters are the slow ones, and while a
#            change is still moving they are noise; run the whole thing
#            before the push.
#   --heavy  after the library step, the heavy models as well, against
#            their own floors in scripts/heavy_floor.sh. Twenty minutes
#            more, and wanted by a change to the solver: the main pass
#            does not see the heavy models, push CI does not run them,
#            and a regression in `Spice3.Examples.Oscillator` went
#            twenty hours and three pushes before the nightly run
#            caught it. Without the flag the step is named as skipped.
#
# The library directory defaults to `.msl` at the root of the tree,
# which is where every honest measurement of this project was taken,
# and after that to where `library add` puts it. Where there is none
# the floor check says so and is skipped rather than failing: the
# floors are about a library this machine may not have. A skipped step
# is named in the summary, and the summary then says what was checked
# rather than claiming everything was.
set -uo pipefail
cd "$(dirname "$0")/.."

quick=0
heavy=0
library=""
for argument in "$@"; do
  case "$argument" in
    --quick) quick=1 ;;
    --heavy) heavy=1 ;;
    -*) echo "unknown option: $argument" >&2; exit 2 ;;
    *) library="$argument" ;;
  esac
done

if [ -z "$library" ]; then
  if [ -d .msl ]; then
    library=".msl"
  else
    library="${XDG_DATA_HOME:-$HOME/.local/share}/oxidelica/libraries/Modelica"
  fi
fi

# Every failure is reported at the end rather than stopping the run.
# A push is held up by the whole list, and finding out about one thing
# at a time is what makes this slow enough to skip.
failures=()
skipped=()

step() {
  local name="$1"
  shift
  printf '\n\033[1m== %s\033[0m\n' "$name"
  if "$@"; then
    printf '\033[32mok\033[0m: %s\n' "$name"
  else
    printf '\033[31mFAILED\033[0m: %s\n' "$name"
    failures+=("$name")
  fi
}

# A linter this machine does not have is not a failure of the commit.
# CI has them all; here they are reported as unchecked so the gap is
# visible rather than silently passing.
optional_step() {
  local name="$1"
  local tool="$2"
  shift 2
  if ! command -v "$tool" > /dev/null 2>&1; then
    printf '\n\033[1m== %s\033[0m\n\033[33mskipped\033[0m: `%s` is not installed\n' "$name" "$tool"
    skipped+=("$name")
    return
  fi
  step "$name" "$@"
}

examples_all_simulate() {
  cargo build --release -p oxidelica-cli || return 1
  local status=0
  for model in examples/*.mo; do
    if ! ./target/release/oxidelica simulate "$model" > /dev/null 2>&1; then
      echo "FAILED: $model"
      status=1
    fi
  done
  return $status
}

# The desk's band on the ratio of the two halves, read by the library
# step and seen red and green by time_band_judges; the arithmetic is at
# the library step.
#
# The floor is 0.88 since 2026-11-09, an edge named by Roman and not
# computed here. It was lowered with the settling fix, which makes the
# run half cheaper and the flatten half dearer, so the ratio falls
# without any model being lost. Witnesses, seven quiet desk prints of
# the judge on 2026-11-09 (/tmp/m622/ratio{1,2,_head}.txt and
# /tmp/m623/{h1,r1,h2,r2}.txt, binaries md5 ae40a54f head and 595f9d3c
# fix): head 1.054 / 1.032 / 1.021, fix 0.940 / 0.935 / 0.923 / 0.933.
# The fix stands 4.9% over the edge (0.923 / 0.88) against an
# instrument noise of 0.53% and an evening drift near 1% a print.
# 0.96 comes back when a delta over one fixed set of models replaces
# this band, as library_floor.sh:2680-2686 already asks.
DESK_RATIO_LOW=0.88
DESK_RATIO_HIGH=1.44

# The desk's base for the delta (library_floor.sh, delta_advisory), the
# middle of two quiet prints of the head 160e151 on 2026-11-10, binary
# md5 eac0297e: /tmp/m626/drift3.txt 3941 / 3923 and drift4.txt
# 3950 / 3897 ms per model (flatten / run). Two points give a
# two-point spread of 0.8% in the ratio, not a band; a third is owed.
# The base is wound like a floor: a commit that moves the ratio on
# purpose moves the base in the same commit with the print as its
# witness, and a move nobody meant is what the delta is there to see.
# It is the desk's base only and is never set for the runner.
DESK_BASE_FLATTEN_MS=3945
DESK_BASE_RUN_MS=3910
DESK_BASE_FROM="desk, head 160e151, binary md5 eac0297e, /tmp/m626/drift3.txt and drift4.txt"

library_floor() {
  if [ ! -d "$library" ]; then
    return 0
  fi
  cargo build --release -p oxidelica-cli || return 1
  # The desk's own band on the ratio of the two halves; the numbers and
  # the arithmetic behind them are at the band in library_floor.sh.
  #
  # Moved on 2026-10-06 from 0.97..2.10 by the factor the carving of
  # five refused giants put on the ratio, the same width kept. All five
  # were dearer to run than to flatten (run / flatten s, quiet desk,
  # /tmp/m364/slow.txt): DynamicPipesWithTraceSubstances 495.1 / 161.0,
  # BranchingPipes12 324.8 / 39.4, R134a2 193.0 / 91.7,
  # DynamicPipesAndFittings 47.1 / 151.0, R134a1 114.7 / 79.3. Quiet
  # desk passes with them gave 1.034 and 1.025 (/tmp/m364/slow.txt,
  # floor2.txt), the pass without them 0.848 (/tmp/m365/main_new.txt):
  # 0.848 / 1.030 = 0.823, and 0.97 and 2.10 times that are 0.80 and
  # 1.73. The runner's band in library_floor.sh is not moved.
  #
  # Not moved on 2026-10-12, when the runner's band was recentred from
  # 0.70..1.50 to 1.45..2.02. Moving the desk by the factors it stood at
  # (1.143 and 1.153) would give 1.66..2.33, and the factors assumed the
  # desk runs above the runner, which on the present code it does not:
  # three desk passes of it gave 1.333 (/tmp/m401/preflight.txt), 1.346
  # (/tmp/m402/preflight.txt) and 1.303 (/tmp/m403/corpus_off.txt),
  # against the runner's 1.725 and 1.745. Under 1.66 every one of them
  # would have been red. 0.80..1.73 holds all three, and the desk's
  # edges wait for more desk points rather than being fitted to three.
  #
  # Set again on 2026-10-12 from the desk's own measurement, on Roman's
  # word, and no longer as a factor of the runner's band: the desk ran
  # above the runner once and runs below it now, so a factor carried
  # from one to the other was carrying a fact that had changed. Eight
  # refused giants were carved out of the main pass that day; the quiet
  # desk pass after the carving, one binary built from ffd3bcb with this
  # list (/tmp/m405/main2.txt), printed 3834ms per model flattening and
  # 3655ms running, a ratio of 0.953. The edges keep the room the
  # runner's band keeps, 18.9% under and 15.8% over: 0.953 / 1.189 =
  # 0.80 and 0.953 x 1.158 = 1.10. They stand on ONE point. Desk passes
  # of one code have spread from 1.30 to 1.69 before, wider than that
  # room, so the next desk passes are the next points, and one that
  # falls outside on right counts has the edges worked over all of them
  # rather than fitted to it.
  #
  # Worked over on 2026-10-14, when two such passes fell outside. Both
  # were alone on a quiet desk, the eight giants carved out, every count
  # right: the compiler of 8be7895 with its two switches off printed
  # 3747ms flattening and 4305ms running, 1.149
  # (/tmp/m419/floor_alone_off.txt), and with them on 3675ms and 4324ms,
  # 1.177 (/tmp/m419/floor_alone.txt). A third pass of 1.106 is left
  # out: it shared the desk with a corpus pair, which is not this set's
  # weather. Passes of 1.30..1.35 from before the carving are another
  # set and left out too. Over the three points 0.953, 1.149 and 1.177
  # the edges keep the same room: 0.953 / 1.189 = 0.80 under the lowest
  # and 1.177 x 1.158 = 1.36 over the highest.
  #
  # Worked over again on 2026-10-16, on Roman's word, because the edge
  # of 0.80 had gone red on every quiet preflight since the compiler
  # changed, which made the guard dead: a true fall of the ratio could
  # no longer be told from the normal. Five quiet passes on the new
  # compiler, each read from its preflight file (flattening ms, running
  # ms, ratio): m431/lib2 4278 3362 0.786, m431/pf_full 4295 3329
  # 0.775, m432/pf 4234 3251 0.768, m433/pf 4142 3236 0.781, m435/pf
  # 4227 3252 0.769. Two passes are left out as a busy desk:
  # m431/pf_quick 5034 3387 0.673 and m429/pf 6592 4186 0.635. The sign
  # that sorts them is flattening: the quiet five sit at 4142..4295, a
  # spread of 3.6%, and a busy desk swells flattening by 19% and 56%
  # while barely moving the run. Two of the five came after the
  # question was put on three and fell inside the old spread, so the
  # base will not grow firmer. The room is the same, kept by dividing
  # and multiplying as both precedents did: 0.768 / 1.189 = 0.6459,
  # written 0.64, and 0.786 x 1.158 = 0.9102, written 0.91. A busy desk
  # of 0.673 now passes and one of 0.635 stays red; catching a busy desk
  # is not this band's work. The runner's band of 0.70..1.50 is not
  # touched.
  #
  # Worked over on 2026-11-03, on Roman's word that each machine is
  # held by its own print. The edges of 2026-10-16 had been red on every
  # desk preflight since the compiler moved again. Eleven desk prints
  # of the ratio, each read from its file (`run/flatten ratio is`):
  # m543/preflight 1.231, m544/pre 1.230, m546/preflight 1.179,
  # m547/preflight 1.182, m549/preflight 1.242, m550/pre 1.219,
  # m551/preflight 1.248, m552/preflight 1.224, m564/pf_quick 1.164,
  # m564/preflight 1.148, m566/preflight 1.143. The room is the same as
  # every precedent: 1.143 / 1.189 = 0.9613, written 0.96 (history: the
  # floor is now 0.88, Roman's word of 2026-11-09, see DESK_RATIO_LOW), and
  # 1.248 x 1.158 = 1.4452, written 1.44. The desk's centres of work
  # come with FLOOR_DESK=1, at the centres in library_floor.sh. The
  # runner's band and centres are not touched.
  #
  # The two passes and the judge, one after the other, as CI runs them
  # side by side since 2026-10-13: the desk measures the same set and
  # judges the same sum, so a fault in the cut shows here first. The
  # reports stay in a directory of their own for whoever wants the names.
  local reports
  reports="$(mktemp -d)"
  ./scripts/library_floor.sh --pass main "$library" "$reports/main.txt" || return 1
  ./scripts/library_floor.sh --pass dear "$library" "$reports/dear.txt" || return 1
  echo "the two reports: $reports"
  FLOOR_DESK=1 RATIO_LOW="$DESK_RATIO_LOW" RATIO_HIGH="$DESK_RATIO_HIGH" \
    DELTA_BASE_FLATTEN_MS="$DESK_BASE_FLATTEN_MS" DELTA_BASE_RUN_MS="$DESK_BASE_RUN_MS" \
    DELTA_BASE_FROM="$DESK_BASE_FROM" ./scripts/library_floor.sh --judge "$library" \
    "$reports/main.txt" "$reports/dear.txt"
}

# The guard on the cut of the library job, seen red on a name left out,
# a name read twice and a name from outside, and green on a true cut.
partition_judges() {
  local sets
  sets="$(mktemp -d)"
  printf 'A\nB\nC\n' > "$sets/all"
  printf 'A\n' > "$sets/main"
  printf 'B\nC\n' > "$sets/dear"
  printf 'B\n' > "$sets/short"
  printf 'A\nB\nC\n' > "$sets/twice"
  printf 'B\nC\nD\n' > "$sets/outside"
  ./scripts/library_floor.sh --partition-check "$sets/all" "$sets/main" "$sets/dear" || return 1
  local wrong
  for wrong in short twice outside; do
    ./scripts/library_floor.sh --partition-check "$sets/all" "$sets/main" "$sets/$wrong" > /dev/null && return 1
  done
  rm -rf "$sets"
  return 0
}

# The band on the ratio of the two halves' times, seen to fire on
# either side and to stay quiet in the middle. The library step is the
# only other place it runs, and there it is green whenever the pass
# is, so without this nobody would see it red until it mattered.
time_band_judges() {
  # The runner's band: under its floor, past its top, and the median
  # of the 46 jobs it was first set over inside.
  ./scripts/library_floor.sh --ratio-check 1000 500 > /dev/null && return 1
  ./scripts/library_floor.sh --ratio-check 1000 2000 > /dev/null && return 1
  ./scripts/library_floor.sh --ratio-check 1000 1045 || return 1
  # And the desk's band, at the two ends of what the desk has measured
  # and past either end of it.
  # Inside: the lowest and the highest quiet desk points the edges of
  # 2026-10-16 stand on, 0.768 and 0.786, and one between, 0.775.
  # Outside: 0.600 under the floor, and 0.953, the quiet point of the
  # old compiler - should the ratio go back to that shape, the band has
  # to go red.
  # Inside since 2026-11-03: the lowest and highest desk prints the
  # edges stand on, 1.143 and 1.248, and one between, 1.164. Outside:
  # 0.786, the quiet point of the compiler before, and 1.500.
  # Inside since 2026-11-09, when the floor went to 0.88: the quiet
  # prints of the settling fix, 0.923 and 0.940. 0.786 stays outside,
  # now 10.7% under the floor rather than 18%.
  local desk="env RATIO_LOW=$DESK_RATIO_LOW RATIO_HIGH=$DESK_RATIO_HIGH ./scripts/library_floor.sh --ratio-check"
  $desk 1000 1143 > /dev/null || return 1
  $desk 1000 1248 > /dev/null || return 1
  $desk 1000 1164 > /dev/null || return 1
  $desk 1000 923 > /dev/null || return 1
  $desk 1000 940 > /dev/null || return 1
  $desk 1000 786 > /dev/null && return 1
  $desk 1000 1500 > /dev/null && return 1
  return 0
}

# The delta line, seen in its five shapes without a library pass: a
# quiet print of the head speaks small, a ratio 6% off names the guess,
# both halves x1.5 at the same ratio are named by the halves, a zero
# flatten says it cannot divide, and with no base there is no line.
delta_judges() {
  local d="env DELTA_BASE_FLATTEN_MS=$DESK_BASE_FLATTEN_MS DELTA_BASE_RUN_MS=$DESK_BASE_RUN_MS DELTA_BASE_FROM=test ./scripts/library_floor.sh --delta-check"
  local out
  out="$($d 3941 3923)" || return 1
  echo "$out" | grep -q '^delta: ratio' || return 1
  echo "$out" | grep -q 'past' && return 1
  $d 3945 4150 | grep -q 'ratio moved past' || return 1
  out="$($d 5918 5865)" || return 1
  echo "$out" | grep -q 'ratio moved past' && return 1
  echo "$out" | grep -q 'both halves grew' || return 1
  $d 0 3910 | grep -q 'cannot be divided' || return 1
  out="$(env -u DELTA_BASE_FLATTEN_MS -u DELTA_BASE_RUN_MS ./scripts/library_floor.sh --delta-check 3941 3923)" || return 1
  [ -z "$out" ] || return 1
  return 0
}

# The band on the counts of work, seen the same way: five percent
# either side of the written number fires, inside it is quiet, and a
# band of its own in parts per million is honoured.
work_band_judges() {
  ./scripts/library_floor.sh --work-check 640 605 > /dev/null && return 1
  ./scripts/library_floor.sh --work-check 570 605 > /dev/null && return 1
  ./scripts/library_floor.sh --work-check 630 605 || return 1
  ./scripts/library_floor.sh --work-check 1003000 1000000 2000 > /dev/null && return 1
  ./scripts/library_floor.sh --work-check 1001000 1000000 2000 || return 1
  return 0
}

step "Formatting" cargo fmt --all -- --check
step "Clippy" cargo clippy --workspace --all-targets -- -D warnings
# Debug, no --release, the way CI runs them. Not a saving of time: a
# recursion deep but not infinite survives the main thread's eight
# megabytes and aborts a test thread's two, so a release binary run on
# the main thread returns where a debug test on a small stack dies.
# ce42b15 shipped green from a hand-run `cargo test --release` and
# SIGABRTed CI on the next command. Measure where the failure lives.
step "Tests" cargo test --workspace
step "The examples all simulate" examples_all_simulate
step "The time band fires on both sides" time_band_judges
step "The delta speaks, and is silent without a base" delta_judges
step "The work band fires on both sides" work_band_judges
step "The cut of the library job is guarded" partition_judges

if [ -d "$library" ]; then
  step "The standard library still reads" library_floor
else
  printf '\n\033[1m== The standard library still reads\033[0m\n'
  printf '\033[33mskipped\033[0m: no library at %s\n' "$library"
  skipped+=("The standard library still reads")
fi

heavy_name="The heavy models hold their floors"
if [ "$heavy" -eq 1 ] && [ -d "$library" ]; then
  step "$heavy_name" ./scripts/heavy_floor.sh "$library"
else
  printf '\n\033[1m== %s\033[0m\n' "$heavy_name"
  if [ "$heavy" -eq 1 ]; then
    printf '\033[33mskipped\033[0m: no library at %s\n' "$library"
    skipped+=("$heavy_name")
  else
    # Not run is not unchecked: push CI does not run these either, so
    # the step is named in the summary without changing its verdict.
    printf '\033[33mnot run\033[0m: wanted by solver changes, run with --heavy\n'
  fi
fi

if [ "$quick" -eq 0 ]; then
  optional_step "Coverage, the floor is 92.5% of lines" cargo-llvm-cov \
    ./scripts/coverage.sh --summary-only
  optional_step "TOML" taplo taplo fmt --check --diff
  optional_step "Spelling" typos typos
  # The workflow checks the prose as well as the code, and a green
  # commit here that went red there was a formatter this script never
  # ran: the same versions the workflow names, so what passes here
  # passes there.
  # The same files CI lints, which is the ones git tracks: the running
  # note left for fable is ignored by git and is a journal rather than
  # a document - one heading per round, the same headings every round -
  # so linting it here made the preflight red for something CI never
  # sees, and a check that is always red is a check nobody runs.
  # A library fetched to be measured against is the same case from the
  # other end. `.msl/` is the standard library, git-ignored and written
  # by somebody else, and its release notes carry a thousand duplicate
  # headings: a desk that had run `library check` went red on prose no
  # commit here can mend, while CI - which has no such directory when
  # it lints - stayed green. Whatever the check ignores it ignores in
  # both halves, or the two verdicts are not the same verdict.
  optional_step "Markdown, JSON and YAML" npx \
    npx --yes prettier@3.9.6 --check "**/*.{md,json,yaml,yml}" --log-level warn \
    --ignore-path .gitignore
  optional_step "Markdown style" npx \
    npx --yes markdownlint-cli2@0.23.2 "**/*.md" "!target" "!.msl" \
    "!QUESTION_FOR_FABLE.md" "!QUESTION_FOR_FABLE_ARCHIVE.md"
  step "No Cyrillic outside the files that may hold it" python3 scripts/check_cyrillic.py
  optional_step "Unused dependencies" cargo-machete cargo machete
  step "Documentation builds, and every public item has some" \
    env RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps
fi

printf '\n\033[1m== summary ==\033[0m\n'
if [ "$heavy" -eq 0 ]; then
  printf '\033[33mnot run\033[0m: %s (--heavy)\n' "$heavy_name"
elif [ -d "$library" ] && [[ ! " ${failures[*]:-} " == *"$heavy_name"* ]]; then
  printf '\033[32mpassed\033[0m: %s\n' "$heavy_name"
fi
unchecked=0
for name in "${skipped[@]:-}"; do
  if [ -n "$name" ]; then
    printf '\033[33munchecked\033[0m: %s\n' "$name"
    unchecked=$((unchecked + 1))
  fi
done
if [ "${#failures[@]}" -eq 0 ]; then
  # A summary that says more than was measured is the quiet number this
  # project hunts in the compiler: "everything passes" printed under a
  # step that never ran reads exactly like a full pass.
  if [ "$unchecked" -gt 0 ]; then
    printf '\033[32meverything checked here passes\033[0m; %d step(s) unchecked above - CI decides those\n' "$unchecked"
  elif [ "$quick" -eq 1 ]; then
    printf '\033[32mthe fast checks pass\033[0m - run without --quick before pushing\n'
  else
    printf '\033[32meverything CI checks on one platform passes\033[0m\n'
  fi
  exit 0
fi
for name in "${failures[@]}"; do
  printf '\033[31mfailed\033[0m: %s\n' "$name"
done
exit 1
