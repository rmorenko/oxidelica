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
# The headroom is wide on purpose, and it was measured too narrow
# once. The first pair was set about two thirds over the dearest run
# then seen, on a belief that two runs of one code differ by a
# seventh. They differ by far more than that. One commit was run
# twice on this machine, once by a push and once by the schedule an
# hour later, with the same binary over the same 1043 models: 5939ms
# per model flattening on the green run and 7686ms on the red one, a
# spread of 29% with nothing between them but which runners the day
# handed out. The desk is wilder still - two whole-corpus passes of
# one binary reporting the same 471 models printed 1725ms and 3412ms,
# a factor of two.
#
# So the ceilings are set above the noise rather than above the
# dearest sample, which is what makes them readable: a firing is then
# news instead of a coin toss. This is not a benchmark and a third of
# a percent is not a finding. It is a trap for the kind of regression
# that was paid for once: an index reduction that took the run half
# from 591s to 3153s, a factor of five, went unnoticed for two days
# and then killed two build machine runs on the ninety minute cap. A
# factor of five still clears these by a mile, and the noise band no
# longer wakes them.
#
# `--slow N` on the check itself is the other half of this pair: the
# ceiling says the compiler got slower, and the list of the dearest
# models by half says where, from the same run rather than from two
# more.
#
# A handful of models are not measured here at all. The names are in
# `scripts/heavy_models.txt` with what each cost and when: three
# `Spice3` benchmarks took 2693s of a 3757s run half between them,
# seventy-two percent of the time on three names, and what they measure
# is the speed of the solver rather than how much of the library reads.
# They are held to floors of their own by `scripts/heavy_floor.sh` on a
# schedule, so nothing is hidden by the carving - and the floors below
# came down by exactly what moved:
#
#   flatten 831 = 828 here + 3 in the scheduled run
#   run     492 = 492 here + 0 in the scheduled run
#   runnable flatten 733 = 732 here + 1 in the scheduled run
#   runnable run     461 = 461 here + 0 in the scheduled run
#
# The run halves did not move because none of the three runs yet: what
# they cost is spent reaching a refusal. That is written down because a
# number that did not fall is as easy to misread a week later as one
# that did.
#
# A fourth name joined that list on 2026-09-21, and this one is carved
# out of the flatten half rather than the run half:
# `Mechanics.MultiBody.Examples.Loops.EngineV6` cost 270.1s of a 2032s
# flatten half, 13.3% on one name, with the next dearest model three
# times cheaper. It flattens and does not run, so only the two flatten
# floors move, and the arithmetic is written out because a number that
# fell for a good reason and a number that fell from a regression look
# identical a week later:
#
#   flatten 867 = 866 here + 1 in the scheduled run
#   runnable flatten 752 = 751 here + 1 in the scheduled run
#   run 536 and runnable run 502 are unmoved: EngineV6 does not run,
#   so nothing left the run half with it.
#
# Both numbers are from one pass of one binary over the corpus with
# EngineV6 already in `heavy_models.txt` (/tmp/m218/corpus.txt) - the
# check reported 867 flatten and 752 runnable flatten - and the
# scheduled half from /tmp/m218/heavy.txt.
#
# Two more names joined that list on 2026-09-22, both from
# `ModelicaTest.Fluid.TestComponents.Pipes`, and both for the same
# reason as the others: what they cost is spent reaching a wall that
# is already on the map. `DynamicPipeWithNominalLaminarFlow` cost at
# least 293s of a 1989s run half - 14.7% on one name, with the next
# dearest at 147.6s, twice cheaper - and what it reaches is the parked
# IF97 wall, `pipeN20.mediums[3].d` bound to a `waterBaseProp` call
# the compiler will not evaluate before the run.
# `DynamicPipeEnergyConservationCheck2` cost at least 67s, nearly all
# of it in the flatten half, and stands at the same wall. Both flatten
# and neither runs, so again only the flatten floors move:
#
#   flatten 867 = 865 here + 2 in the scheduled run
#   runnable flatten 752 = 750 here + 2 in the scheduled run
#   run 540 and runnable run 506 are unmoved: neither of the two runs.
#
# The main numbers are from /tmp/m229/main.txt and the scheduled half
# from /tmp/m229/heavy.txt, both from one build of one binary.
#
# Its namesake without the `2` is a different matter and stays in the
# main run at 199s: a physical check is not a benchmark however dear.
#
# Usage: scripts/library_floor.sh <library directory>
set -euo pipefail

FILES_FLOOR=2671
FLATTEN_FLOOR=865
# The two run floors came down by one, and the one is named: giving a
# record constructor called with no arguments the values its `extends`
# stated took `Modelica.Thermal.FluidHeatFlow.Examples.WaterPump` out
# of the run list. It asked for water and had been given the base
# record's placeholder of one for every property - a heat capacity of
# one where water's is 4177 - and it ran on that wrong number. With
# water's own figures it reaches the parked `do not mention` wall that
# its siblings stand at, which is where a model asking for water
# belongs. A number that fell for a good reason and a number that fell
# from a regression look identical a week later, so:
#
#   run          530 = 531 before, less WaterPump
#   runnable run 498 = 499 before, less WaterPump
# Judging a torn block's divisor by what it comes to at the starts,
# rather than by whether it mentions a name that starts at zero, moved
# the run pair by three and by two. Named both ways, from
# /tmp/m209/ran_before.txt against /tmp/m209/ran_after.txt:
#
#   arrived  SaturatedInductor, ComparisonQuasiStatic,
#            NonLinearInductor, Analog.Examples.Utilities.Transistor
#   left     QuadraticCoreAirgap, which now stands at
#            `underdetermined algebraic loop ["leakage.Phi", ...]`
#
# The departure is the plan's doing and not the Jacobian's: with
# OXIDELICA_NO_ROW_SCALING it refuses the same way, and with
# OXIDELICA_DIVISOR_BY_MENTION it runs. A block that grew smaller can
# come out nearer to square than the check likes, which is the price of
# four, and it is written here so that a number that fell for a good
# reason does not read as a regression a week later.
#
#   run          533 = 530 before, plus four, less QuadraticCoreAirgap
#   runnable run 500 = 498 before, plus the two of those four that are
#                runnable examples
# Not dividing by a state that starts at zero brought one more, and
# the runnable pair did not move because it is not a runnable example.
# Named from one binary, /tmp/m210/before_list.txt against
# /tmp/m210/after_list.txt, with the fix behind
# OXIDELICA_NO_STATE_DIVISOR so that only the rule differs:
#
#   arrived  MultiBody.Examples.Elementary.
#            PointGravityWithPointMasses2.SystemWithStandardBodies
#   left     nothing
#
#   run          534 = 533 before, plus one
#   runnable run 500, unmoved
#
# Putting a Newton step's columns in their own units brought one more,
# and this time the runnable pair moves with it. An enthalpy carried by
# a mass flow that is zero at rest gives a column at 1e-24 beside a
# volume flow at 1e-4, and pivots judged against 1e-14 flat called it
# dead; scaled, the block is plainly invertible, and the step it yields
# is damped from its first use because a pivot that small says the
# block is nearly flat that way. Named from one binary,
# /tmp/m212/chainoff.txt against /tmp/m212/chain.txt, with the rule
# behind OXIDELICA_NO_COLUMN_UNITS:
#
#   arrived  Modelica.Thermal.FluidHeatFlow.Examples.PumpAndValve
#   left     nothing
#
#   run          535 = 534 before, plus one
#   runnable run 501 = 500 before, plus one
#
# And one more, from the line search refusing to count a fall it
# could buy only by cutting the step to a sliver (/tmp/m214/on2.ran
# against /tmp/m214/off.ran, the two halves from one binary with the
# rule behind OXIDELICA_SLIVER_STEPS):
#
#   arrived  Modelica.Electrical.Machines.Examples.Transformers.Rectifier12pulse
#   left     nothing
#
#   run          537 = 536 before, plus one: an exponent that was two
#                 literals divided is folded before being called live,
#                 and a dry air medium runs on the strength of it
#   runnable run 503 = 502 before, plus the same one
#
# And two more, from a `noDerivative` over a Real input being left
# without a rule rather than read as though it named a record. The
# diff was taken from two binaries of one machine - the list after in
# /tmp/m226f/ran_after_fable.txt against the list before in
# /tmp/m226f/ran_before.txt, the latter built in a worktree on
# 7b76dd0 - and confirmed by a second run of the same tree
# (/tmp/m226g/corpus_mine.txt, 867 flatten and 539 run, 752 and 505
# runnable):
#
#   arrived  Modelica.Fluid.Examples.DrumBoiler.DrumBoiler
#            ModelicaTest.Fluid.TestComponents.Sensors.TestFlowRate
#   left     nothing
#
#   run          539 = 537 before, plus the two above
#   runnable run 505 = 503 before, plus the same two, both of them
#                 examples by the Example icon
#
# Flattening does not move: both models flattened before as well.
#
# Both run floors go up by one, and the one is named: the
# initialisation now knows the derivative of a state index reduction
# demoted, which is the dummy the plan already computes, so
# Modelica.Blocks.Examples.FilterWithDifferentiation runs where it
# refused with `der(Bessel.x[2])`.
#
#   run          540 = 539 before, plus that one model
#   runnable run 506 = 505 before, plus the same model, an example by
#                     its experiment annotation
#
# Flattening does not move: it flattened before as well. The diff of
# the run lists is exactly that one name arriving and none leaving.
#
# And again for the same shape of reason, this time in the solver's
# finite-difference step: a column of the Jacobian that came back all
# zeros is now asked again from further away before it is believed
# dead, because a coefficient too small for the residual to resolve
# subtracted away to an exact zero. `CCCV_Cell` runs on it.
#
#   run          541 = 540 before, plus that one model
#   runnable run 507 = 506 before, plus the same model
#
# Flattening is unmoved on both counts, and the diff of the run lists
# taken from one binary with the fix switched off and on is that one
# name arriving and none leaving.
#
# A step further along the same road. Asking a dead column again from
# further away told the solver which kind it was; taking the slope it
# found and putting it into the matrix lets the block be solved. The
# two kinds stay apart: a column that does not move from a unit away
# keeps its refusal in the words it always had, because there the
# equations really have lost the unknown.
#
# The slope is asked in both directions and kept only where the two
# agree, which is what decides whether a block has one solution here
# or two. That narrowing took a model back off the list rather than
# adding one: `TestSuddenExpansion` was answered by the first
# direction tried and is now refused by its own ambiguity, so two
# models arrive and not three - a thyristor bridge and `WaterPump`.
# The whole of the repair lives past a step that already came back as
# nothing, so a model that ran cannot be touched by it.
#
#   run          543 = 541 before, plus those two
#   runnable run 509 = 507 before, plus the same two
#
# Flattening is unmoved on both counts, and the diff of the run lists
# taken from one binary with the repair switched off and on is those
# two names arriving and none leaving (/tmp/m243/off.txt and
# /tmp/m243/on.txt: 865/541 and 750/507 against 865/543 and 750/509).
#
# The whole of `Modelica.Electrical.Digital` that was standing at the
# event iteration arrives at once: thirteen models, because the
# `when initial()` inside the inertial delay no longer fires on a
# round where the definitions behind it still hold the values from
# before the event. Six of the thirteen are `Examples.Utilities`
# helpers rather than examples proper, which is why the two run
# floors move by different amounts.
#
#   run          556 = 543 before, plus the thirteen
#   runnable run 516 = 509 before, plus the seven that are examples
#
# Flattening is unmoved on both counts - all thirteen flattened
# already and refused in the run half. The diff of the run lists,
# taken from one tree built twice (/tmp/m245/before.txt and
# /tmp/m245/after.txt: 865/543 and 750/509 against 865/556 and
# 750/516), is those thirteen names arriving and none leaving.
#
# A variable typed by an enumeration started at zero, which is not a
# position any literal has, and a table read at it fell through the
# chain of `if index == k` a run-time subscript becomes, to a value
# that is no number. Ten of the thirteen that had just arrived were
# filling their results with NaN and being counted as having run -
# `Counter` held 284 columns of them out of 323. Starting such a
# variable at its first literal, and seeding what `pre` of it was
# worth before the run began from the same place, takes the NaN out.
#
#   run          557 = 556 before, less four, plus five
#   runnable run 515 = 516 before, less four, plus three
#
# The four that left - `Counter`, `Counter3`, `FlipFlop`,
# `Multiplexer` - are four of the ten that were filling their rows
# with NaN, and what they meet now is the chatter guard: a gate that
# genuinely oscillates raises more than ten thousand events inside one
# output interval. A run that refused is worth more than a run that
# answered with no number, so the fall of four is the point of the
# change rather than its price. The five that arrived -
# `Adder4`, `HalfAdder`, `Utilities.FullAdder`, `Utilities.HalfAdder`,
# `VectorDelay` - were standing at a fixed start the NaN made
# impossible, and three of them are runnable examples.
#
# Flattening is unmoved on both counts. The diff of the run lists,
# taken from one tree built twice (/tmp/m245/a.ran against
# /tmp/m246/a.ran: 865/556 and 750/516 against 865/557 and 750/515),
# is those four names leaving and those five arriving.
RUN_FLOOR=557
RUNNABLE_FLATTEN_FLOOR=750
RUNNABLE_RUN_FLOOR=515
# Every file of the library parses. This is a ceiling reached rather
# than a floor to hold, so it is written as the number left over: one
# file that stops parsing takes its whole tree of classes with it, and
# the counts below would hide that behind a handful of models.
UNREAD_CEILING=0
# Milliseconds per model that reached each half. See the note above
# for why these are the build machine's numbers and not a desk's.
FLATTEN_MS_CEILING=12000
RUN_MS_CEILING=8000

directory="${1:?usage: library_floor.sh <library directory>}"
cd "$(dirname "$0")/.."

# The names as well as the counts: the run half differs between one
# machine and another, and a difference nobody can name is a
# difference nobody can fix. The list goes to a file rather than the
# log, and the log gets the run half of it, which is where the
# machines disagree.
report="$(./target/release/oxidelica library check --list --without scripts/heavy_models.txt "$directory")"
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
