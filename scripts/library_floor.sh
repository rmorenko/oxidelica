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
# Three more names joined that list on 2026-09-24, the moist-air
# examples of `Modelica.Media.Examples.ReferenceAir`: `MoistAir2`
# 377.7s, `MoistAir1` 372.1s and `MoistAir` 369.4s to flatten, 1119s
# between them of a 4192s flatten half - 26.7% on three names, with the
# next dearest at 155.9s, two and a half times cheaper. What they cost
# is the reference air's Helmholtz tables built whole three times over,
# not how much of the library reads. This is the first carving that
# takes a model that RUNS: `ReferenceAir.MoistAir` runs, so the run
# floors come down by one here and the scheduled run holds that one
# under a run floor of its own, which until now stood at zero. A fall
# from 583 to 582 is this and nothing else:
#
#   flatten          917 = 914 here + 3 in the scheduled run
#   run              583 = 582 here + 1 in the scheduled run
#   runnable flatten 802 = 799 here + 3 in the scheduled run
#   runnable run     541 = 540 here + 1 in the scheduled run
#
# The main numbers are from /tmp/m266/main.txt and the scheduled half
# from /tmp/m266/heavy.txt, both from one build of one binary.
#
# Usage: scripts/library_floor.sh <library directory>
set -euo pipefail

FILES_FLOOR=2671
# The flatten floor moved by four, and the four are named: giving a
# constant imported by an enclosing class its number, and letting a
# check written under a condition be counted, brought in
# Modelica.Media.Examples.ReferenceAir.CO2, ConstantPropertyLiquidWater,
# DryAirNasa and LinearWater_pT_Ambient - the last of which flattens
# and does not run, which is why the run floor moves by three and this
# one by four.
#
#   flatten 869 = 865 before, plus CO2, ConstantPropertyLiquidWater,
#                 DryAirNasa and LinearWater_pT_Ambient
#
# And one more, from deciding a loop head the way the body beside it is
# decided:
#
#   flatten 870 = 869 above, plus Modelica.Blocks.Examples.Filter
#
# And five from unrolling a `for` in an `if` branch the run chooses,
# with a flag worked out in a loop kept a flag - the LossyGear chain:
#
#   flatten 875 = 870 above, plus Rotational.Examples.HeatLosses,
#                 LossyGearDemo1, LossyGearDemo2, LossyGearDemo3 and
#                 ModelicaTest.Rotational.TestBearingConversion
#
# Measured from one binary either side of `OXIDELICA_NO_LOOP_IN_BRANCH`
# and `OXIDELICA_NO_BOOL_FOLD` (/tmp/m257/before.txt: 870/568 and
# 755/526; /tmp/m257/after.txt: 875/571 and 760/529), nothing leaving
# either list.
#
# And four from the subscript family and what stood behind it - a colon
# on the left, `scalar` folded, a record handed by name read in order,
# and a record local of a body given its declared fields:
#
#   flatten 879 = 875 above, plus
#                 ModelicaTest.Fluid.TestComponents.Machines.TestLinearPower
#                 and ModelicaTest.Media.TestOnly.R134a_setState_pTX,
#                 R134a_setState_pTX_high_T and R134a_setState_phX
#
# Measured from one binary either side of `OXIDELICA_NO_COLON_WRITE`,
# `OXIDELICA_NO_SCALAR_FOLD`, `OXIDELICA_NO_NAMED_IN_ORDER` and
# `OXIDELICA_NO_RECORD_LOCALS` (/tmp/m258/off.txt: 875/571 and
# 760/529; /tmp/m258/on.txt: 879/572 and 764/530), nothing leaving
# either list.
#
# And five that stood on the order of their declarations: a time table
# handed `table = startTime.table` from a block declared below it, whose
# `:` nothing had measured yet and whose elements were each bound to
# the whole of the other table:
#
#   flatten 884 = 879 above, plus
#                 ModelicaTest.Tables.CombiTimeTable.Test68 to Test72
#
# Measured from one binary either side of `OXIDELICA_NO_SIBLING_SHAPE`
# (/tmp/m259/c2.txt, the binary before the change: 879/572 and 764/530;
# /tmp/m259/on.txt: 884/577 and 769/535), nothing leaving either list.
#
# And two the table-based media series let through, measured from one
# binary either side of its four switches (/tmp/m260/off.txt:
# 884/577 and 769/535; /tmp/m260/on.txt: 886/578 and 771/536),
# nothing leaving either list:
#
#   flatten 886 = 884 above, plus
#                 Modelica.Media.Examples.SolveOneNonlinearEquation.
#                 InverseIncompressible_sh_T, which stops at a parameter
#                 of `s_T` the run cannot evaluate, and
#                 ModelicaTest.Math.TestPolynomials, whose fit now comes
#                 from `dgelsy` written here
#
# And five the conditions of components let through, measured from one
# binary either side of its six switches (/tmp/m261/off2.txt:
# 886/578 and 771/536; /tmp/m261/on2.txt: 891/578 and 776/536),
# nothing leaving either list. None of the five runs yet, so only the
# flatten floors move:
#
#   flatten 891 = 886 above, plus
#                 Machines.Examples.InductionMachines.IMC_DCBraking,
#                 wired by a string handed down from a setting record,
#                 which stops at `der(imc.is[1])` of a non-state;
#                 FundamentalWave...SynchronousMachines.SMEE_DOL, whose
#                 condition reads a sibling declared below it, at a
#                 singular Jacobian in the air gap's loop;
#                 MultiBody.Examples.Systems.RobotR3.FullRobot, whose
#                 condition reads an `outer` one level down, at two
#                 equations for a gear spring's derivative; and
#                 Constraints.SphericalConstraint and Elementary.
#                 PointGravityWithPointMasses2, whose joints draw their
#                 graph branch before the graph is asked, both at a
#                 structurally singular model
#
# And six a medium's constant named bare let through, measured from
# one binary either side of `OXIDELICA_NO_ENCLOSING_MINT`
# (/tmp/m262/off2.txt: 891/578 and 776/536; /tmp/m262/on2.txt:
# 897/579 and 782/537), nothing leaving either list. `reference_p` in
# `u = h - reference_p/d` of the table-based media now keeps its unit:
#
#   flatten 897 = 891 above, plus Incompressible.Examples.TestGlycol,
#                 which runs; TestAllProperties.IncompleteMedia.Glycol47
#                 and Essotherm650, at `1:2` in the walked `s_T`;
#                 TestsWithFluid...Incompressible.Glycol47 and
#                 Essotherm650, at an algebraic loop through
#                 `shortPipe.flowModel.states[1].p`; and
#                 TestValvesIncompressibleReverse, at a
#                 `solveOneNonlinearEquation` of `V2.state_b.T`
#
# And one a member of an array of components let through, measured
# from one binary either side of both `OXIDELICA_NO_MEMBER_SHAPES` and
# `OXIDELICA_INTERFACE_DIGIT` (/tmp/m263/d_off.txt: 897/579 and
# 782/537; /tmp/m263/d_on.txt: 898/579 and 783/537), nothing leaving
# either list and the run list the same to the name. `medium_T[1].Xi`
# now keeps the shape the size table measured for it:
#
#   flatten 898 = 897 above, plus Media.Examples.PsychrometricData,
#                 which flattens and does not run. The medium's own
#                 `reference_T` read in an inherited body moved no
#                 count: LinearColdWater now reads 278.15 - 278.15,
#                 and stands at the next wall, a bare `2*101325` in
#                 `isentropicEnthalpy`
#
# And seven layers of the trace-substance chain, measured from one
# binary either side of all seven switches (OXIDELICA_NO_SLICE_MEMBER_
# SHAPES, _DISABLED_BY_NAME, _ARRAY_SEEDS, _ENCLOSING_RECORDS,
# _LOOP_CONSTANTS, _ELEMENT_STREAMS, _WHEN_VECTORS). The off side
# printed 898/579 and 783/537 and the work counts above to the digit
# (/tmp/m264/off.txt); the on side (/tmp/m264/on.txt) printed:
#
#   flatten 908 = 898 above, plus ten, none lost:
#                 Blocks.Examples.Interaction1,
#                 Fluid.Examples.ControlledTankSystem.ControlledTanks,
#                 StateGraph.Examples.ControlledTanks (a `when` over a
#                 vector named whole), Media.Examples.ReferenceAir.
#                 MoistAir, MoistAir1, MoistAir2, ModelicaTest.Media.
#                 TestOnly.MoistAir (moist air's default enthalpy),
#                 and TestJunctionTraceSubstances, TestMultiPort,
#                 DynamicPipesWithTraceSubstances (the trace-substance
#                 ports)
#
# Then the m265 series, measured as one pair from one binary
# (/tmp/ox265i) with its three switches set and clear
# (OXIDELICA_SPREAD_COPIES, _NO_NAMED_FIELD_LENGTHS,
# _UNGUARDED_RECORD_READS). The off side printed 908/583 and 793/541
# (/tmp/m265/p2/off.txt); the on side (/tmp/m265/p2/on.txt) printed:
#
#   flatten 917 = 908 above, plus nine, none lost:
#                 Fluid.Examples.TraceSubstances.RoomCO2 and
#                 RoomCO2WithControls, ModelicaTest.Fluid.TestComponents.
#                 Fittings.TestMultiPortTraceSubstances, ModelicaTest.
#                 Media.TestsWithFluid.MediaTestModels.Air.MoistAir (all
#                 four the divisor wall), Sources.TestSources and
#                 Sensors.TestTraceSubstances (shapes [] and [1]),
#                 Fluid.Examples.BranchingDynamicPipes, and both
#                 Inverse_sh_TX, of ReferenceAir and of
#                 SolveOneNonlinearEquation (the copy of Brent's method
#                 spread over its array input). None of the nine runs
#                 yet, so the run floors stand.
#
# And flatten 917 = 914 plus three, from /tmp/m267/on2.txt against
# /tmp/m267/off2.txt (one binary, the five switches of the series off
# together; the off side is the floors to the digit). The three are
# Fluid.Examples.HeatingSystem, Media.Examples.R134a.R134a1 and
# R134a2, all runnable examples: a `redeclare function extends` that
# declares no inputs of its own now has its inherited ones read when
# the resolver asks which arguments may be arrays. Nothing left the
# flatten list.
FLATTEN_FLOOR=917
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
#
# The run pair comes down by six and by one, and the six are named.
# A discrete variable that reaches NaN used to be carried untouched to
# the stop time, so the model finished and the check counted it as a
# win while every column it wrote from that point on was not a number.
# Asked after the event has come to rest, six models of
# `Electrical.Digital` say so: `Adder4`, `Utilities.Counter`,
# `Utilities.Counter3`, `Utilities.DFF`, `Utilities.JKFF` and
# `Utilities.RSFF`. Five of the six name `TD1.x_delayed` inside the
# transport delay, so this is one layer rather than six faults. A
# refusal that names the variable is worth more than a row of NaN
# presented as an answer, which is the trade this whole compiler is
# built on, so the fall is the point of the change rather than a
# regression to be held against.
#
#   run          551 = 557 before, less the six
#   runnable run 514 = 515 before, less the one of the six that is a
#                runnable example
#
# Both halves from one binary built once and run twice, with the check
# off and on (/tmp/m247/off.txt: 865/557 and 750/515;
# /tmp/m247/on.txt: 865/551 and 750/514), and the diff of the two run
# lists is exactly those six names and nothing else - nothing outside
# `Digital` was answering with NaN. Flattening is unmoved on both
# counts, as it must be: the check lives in the run half.
#
# Five of the six are back, and by the writer rather than by the
# reader: `delay(u, T)` before anything has been remembered is `u` at
# the start time, and an empty memory used to answer zero instead.
# `Electrical.Digital`'s transport delay indexes a table of logic
# values by that answer and `LogicValues[0]` has no element, so the
# whole of the first delay came out NaN. The sixth, `Adder4`, has a
# wall of its own - a gate's `auxiliary[2]` reaches NaN in the fifth
# round of the initial event - and is left standing.
#
#   run          556 = 551 before, plus the five flip-flops
#   runnable run 514 unmoved: none of the five is a runnable example,
#                and `Adder4`, which is one, still does not run
#
# Measured on /tmp/m248/corpus.txt: 865/556 and 750/514, and the diff
# of the run lists against /tmp/m247/b.ran is exactly `Counter`,
# `Counter3`, `DFF`, `JKFF` and `RSFF` arriving with nothing leaving.
#
# And now the sixth as well, by the language rather than by the gate:
# a branch of an `if` inside a `when` that says nothing about a
# variable leaves it the value it had, which at an event is `pre` of
# it. The algorithm side read that as the type's start, so the
# inertial delay's `y_auxiliary` came out of a tick it did not fire on
# as zero - a logic value no table has - and `AndTable` answered with
# no number a round later. The `if` written among equations had read
# it the right way all along; the two roads to an event now agree.
#
#   run          557 = 556 before, plus `Adder4`
#   runnable run 515 = 514 before, plus `Adder4`, which is a runnable
#                example: it carries `experiment(StopTime=...)`
#   both flatten counts unmoved: the model always flattened
#
# Measured on /tmp/m249/corpus.txt: 865/557 and 750/515, and the diff
# of the run lists (/tmp/m248/b.ran against /tmp/m249/a.ran) is the
# single line `Modelica.Electrical.Digital.Examples.Adder4` arriving
# with nothing leaving.
# And again a road, this time the walk's search for a crossing. An
# indicator reading exactly zero was taken for a relation leaving its
# threshold as the walk left, so the walk stepped a hair along - and
# a digital gate holds zero over a stretch before stepping off it, so
# at the hair the reading was zero still and the same turn was found
# again. Five Digital examples crept forward by 1e-12 at a time from
# t = 0.003 and raised ten thousand events without moving. The hair
# was standing in for the instant zero gives out, and that instant is
# now found by asking.
#
#   run          563 = 557 before, plus the five Digital examples that
#                stood at the one signature - Counter, Counter3,
#                FlipFlop, FullAdder, Multiplexer - and
#                Analog.AD_DA_conversion, which chattered on the same
#                layer at another instant
#   runnable run 521 = 515 before, plus the same six, all of them
#                runnable examples
#   both flatten counts unmoved: all six always flattened
#
# Measured on /tmp/m250/corpus.txt: 865/563 and 750/521, and the diff
# of the run lists (/tmp/m249/a.ran against /tmp/m250/b.ran) is those
# six names arriving with nothing leaving.
#
# And a wired logic node ran once its declared start stopped being read
# as a condition on the first instant. `fixed = true` on a discrete
# name pins what the name was before the first event, not what it is
# at t = 0, so a node that starts at `'Z'` and is at once defined by
# its input was refused for keeping the standard.
#
#   run          564 = 563 before, plus Electrical.Digital.Examples.WiredX
#   runnable run 522 = 521 before, plus the same one, a runnable example
#   both flatten counts unmoved: it always flattened
#
# Measured from one binary either side of an environment switch
# (/tmp/m252/before.txt: 865/563 and 750/521; /tmp/m252/after.txt:
# 865/564 and 750/522), and the diff of the run lists is that one
# name arriving with nothing leaving.
#
# And a record constant brought in by an enclosing class's `import` is
# now read where a function body names it. The walk out of the
# enclosing packages answered with an `f64`, so `import
# Modelica.ComplexMath.j` at the top of a block left `j` inside
# `powerOfJ` with no road at all: the bare name travelled into the flat
# model and met `unknown variable j` a storey lower.
#
#   run          565 = 564 before, plus ComplexBlocks.Examples.ShowTransferFunction
#   runnable run 523 = 522 before, plus the same one, a runnable example
#   both flatten counts unmoved: it always flattened
#
# Measured from one binary either side of `OXIDELICA_NO_IMPORTED_RECORDS`
# (/tmp/m253/before.txt: 865/564 and 750/522; /tmp/m253/after.txt:
# 865/565 and 750/523), and the diff of the run lists
# (/tmp/m253/b.ran against /tmp/m253/a.ran) is that one name arriving
# with nothing leaving.
#
# And the check-only `if` above, whose numbers are set out at
# `FLATTEN_FLOOR`:
#
#   run          568 = 565 before, plus CO2,
#                ConstantPropertyLiquidWater and DryAirNasa
#   runnable flatten 754 = 750 before, plus those three and
#                LinearWater_pT_Ambient, which flattens and does not run
#   runnable run 526 = 523 before, plus the same three, all runnable
#                examples
#
# And the LossyGear chain set out at `FLATTEN_FLOOR`:
#
#   run          571 = 568 above, plus HeatLosses, LossyGearDemo1 and
#                LossyGearDemo3; LossyGearDemo2 and TestBearingConversion
#                flatten and stop at an algebraic loop
#   runnable flatten 760 = 755 below, plus all five, runnable examples
#   runnable run 529 = 526 below, plus the three that run
#
# And the subscript family set out at `FLATTEN_FLOOR`:
#
#   run          572 = 571 above, plus TestLinearPower, whose `p`
#                comes out 23 as the library's own assert asks; the
#                three R134a tests flatten and stop at a slice the
#                run decides inside a walked `dofpT`, and at `sat`
#   runnable flatten 764 = 760 below, plus all four, runnable examples
#   runnable run 530 = 529 below, plus TestLinearPower
#
# And the five time tables set out at `FLATTEN_FLOOR`, all of which run:
#
#   run          577 = 572 above, plus Test68 to Test72
#   runnable flatten 769 = 764 below, plus the same five, runnable
#                examples
#   runnable run 535 = 530 below, plus the same five
#
# And the table-based media series set out at `FLATTEN_FLOOR`:
#
#   run          578 = 577 above, plus TestPolynomials, whose own assert
#                holds the fitted cubic to the one it was built from
#   runnable flatten 771 = 769 below, plus both, runnable examples
#   runnable run 536 = 535 below, plus TestPolynomials
#
# And the bare constant set out at `FLATTEN_FLOOR`:
#
#   run          579 = 578 above, plus TestGlycol, whose `u` comes out
#                h - 101300/d by hand
#   runnable flatten 782 = 776 below, plus all six, runnable examples
#   runnable run 537 = 536 below, plus TestGlycol
#
# And run 583 = 579, plus Interaction1, both ControlledTanks and
# ReferenceAir.MoistAir, from /tmp/m264/on.txt. The run list lost
# nothing.
#
# And run 587 = 582 after the carving above, plus the five of the
# Wagner wall: Media.Examples.MoistAir, PsychrometricData,
# TestOnly.MoistAir, TestMultiPort and TestTraceSubstances, all
# runnable examples (/tmp/m266/on.txt against /tmp/m266/off.txt from
# one binary; the off side is the carving's main pass to the digit).
# Nothing left the run list and the flatten list did not move. Runnable
# run 545 = 540 plus the same five.
#
# And run 589 = 587 plus the two bends of the new fittings,
# NewFittings.Bends.CurvedBend and EdgedBend, both runnable examples,
# whose `dp_small` is worked out by walking the library's pressure
# loss (/tmp/m267/on2.txt against /tmp/m267/off2.txt). Nothing left
# the run list. Runnable run 547 = 545 plus the same two.
#
# And run 590 = 589 plus TestAllProperties.LinearWater_pT_Ambient, a
# runnable example, whose linear fluid now reads its reference state
# through the water package that fills it in (/tmp/m268/on.txt against
# /tmp/m268/off.txt, one binary). Nothing left the run list and the
# flatten list did not move. Runnable run 548 = 547 plus the same one.
RUN_FLOOR=590
# And runnable flatten 755 = 754 above, plus Filter, which is a
# runnable example and flattens without running.
#
# And runnable flatten 776 = 771, plus the five conditions of
# components set out at `FLATTEN_FLOOR`, all runnable examples.
#
# And runnable flatten 783 = 782, plus PsychrometricData, a runnable
# example set out at `FLATTEN_FLOOR`.
#
# And runnable flatten 793 = 783 plus the ten, runnable run 541 = 537
# plus the four, all of them runnable examples (/tmp/m264/on.txt).
#
# And runnable flatten 802 = 793 plus the nine set out at
# `FLATTEN_FLOOR`, all runnable examples (/tmp/m265/p2/on.txt).
#
# And runnable flatten 802 = 799 plus the three of m267 set out at
# `FLATTEN_FLOOR`, runnable run 547 = 545 plus the two bends set out at
# `RUN_FLOOR` (/tmp/m267/on2.txt).
RUNNABLE_FLATTEN_FLOOR=802
RUNNABLE_RUN_FLOOR=548
# Every file of the library parses. This is a ceiling reached rather
# than a floor to hold, so it is written as the number left over: one
# file that stops parsing takes its whole tree of classes with it, and
# the counts below would hide that behind a handful of models.
UNREAD_CEILING=0
# Milliseconds per model that reached each half. See the note above
# for why these are the build machine's numbers and not a desk's.
FLATTEN_MS_CEILING=12000
RUN_MS_CEILING=8000

# The work the check did, counted in steps rather than seconds, and
# held to within five percent of what is written here either way.
#
# The ceilings above are catastrophe traps: the clock over one binary
# and one library has come out twice as long on one desk pass as on the
# next, so a ceiling can only sit far above the noise, and a doubling of
# the work clears it without a word. That happened: flattening went from
# 2619ms to 5455ms per model with no model won, and a person caught it,
# not a check. The steps do not have weather. Two whole passes of one
# binary over the corpus, run side by side on 2026-09-23
# (/tmp/m258/corpus1.txt and corpus2.txt), printed the same six counts
# below to the digit. A seventh, the names looked up, differed by 47 in
# 1.2 billion between the two passes, so it is printed and not held -
# a count that wanders cannot be a ratchet, and a ratchet that fires
# for nothing is one somebody turns off.
#
# Five percent is wide against a count that does not move at all and
# narrow against the doubling it is there for. Going above it is a
# change that made the compiler do more: if models were won by it, the
# numbers are moved in the same commit with the reason beside them;
# if not, that is the regression. Going below it is written down the
# same way, because a count that fell for a good reason and one that
# fell because a pass stopped being reached look alike a week later.
#
# Set from the pass that also moved the floors to 879/572 on
# 2026-09-23 (/tmp/m258/on.txt), with the four switches of that change
# on. The same binary with them off printed 275971 classes, 89508344
# expansions, 1278449 bodies, 31347957 points, 43593540 newton and 324
# jacobians (/tmp/m258/off.txt), so the change itself did 0.3% more
# expansions and 0.07% more bodies for the four models it let through.
# The counts are a desk's; the build machine's run half may count
# differently where a floating point library answers a last digit
# differently, and if it does, the build machine's numbers go here and
# the difference is named.
#
# The flattening counts also depend on where the library stands on the
# disk, and not only on the binary and the library. A model that reads
# its table from `loadResource("modelica://...")` has the absolute path
# worked over by the string functions, and that work grows with the
# path - by its length or by its segments, which has not been told
# apart. The same binary over the same
# revision printed 89763460 expansions and 1279374 bodies from `.msl`
# (/tmp/m259/c1.txt and c2.txt, which agree to the
# digit) and 89881564 and 1292028 from the preflight's default
# `~/.local/share/oxidelica/libraries/Modelica`
# (/tmp/m259/c3-L.txt). Model by model, over the 187 table and
# utilities models that flatten, the difference sits on the table tests
# that read a file, 3192 expansions and 342 bodies apiece on a
# one-dimensional or time table and 4788 and 513 on a two-dimensional
# one, and comes to 105336 of the 118104 expansions over that set. The
# other 12768 are four times 3192 and lie outside it, not yet traced
# by name. So the numbers here are from `.msl`
# under this checkout. The build machine's checkout path is another
# path, and its flattening counts may sit a fraction of a percent off
# these for that reason alone: 27 characters more moved them 0.13% and
# 0.99%, inside the five percent the ratchet allows.
#
# Refreshed with the floors, from the same run that set them
# (/tmp/m260/on.txt). The same binary with the table-media switches off
# printed 275990 classes, 89768564 expansions, 1279649 bodies and
# 31348264 points (/tmp/m260/off.txt), so the series itself did 1.4%
# more expansions and 0.9% more bodies: the fits of the table-based
# media are worked out wherever a medium of theirs is named, and the
# models that name one are more than the two that came in.
#
# Refreshed again with the floors, from the run that moved them
# (/tmp/m261/on2.txt). The same binary with the six switches of the
# conditions of components off printed exactly the numbers above
# (/tmp/m261/off2.txt), so the series did 1.9% more classes, 0.4% more
# expansions and 0.4% more bodies: the five models that flatten now
# are machines and multi-body systems, and each is built whole.
#
# Refreshed again with the floors, from the run that moved them
# (/tmp/m262/on2.txt). The same binary with the enclosing mint off
# printed exactly the numbers of m261 (/tmp/m262/off2.txt), so the
# series did 0.004% more expansions and 0.09% more bodies: the six
# models that flatten now are small media tests.
#
# Refreshed again with the floors, from the run that moved them
# (/tmp/m263/d_on.txt). The same binary with both switches of m263 off
# printed exactly the numbers of m262 (/tmp/m263/d_off.txt), so the
# series did 0.12% more classes, 0.1% fewer expansions and 0.24% more
# bodies. PsychrometricData is built whole now; which of the two
# changes moved the expansions was not measured apart.
#
# Refreshed again with the floors, from /tmp/m264/on.txt. The off side
# of the same binary printed the numbers of m263 above to the digit
# (/tmp/m264/off.txt), so the series did 0.16% more classes, 1.6% more
# expansions, 7.1% more bodies, 2.9% more points and 2.1% more Newton
# steps. Nearly all of it is ten models built whole, and three of them
# are dear: the ReferenceAir moist-air examples cost about 340s each to
# flatten and DynamicPipesWithTraceSubstances 120s, measured one at a
# time (/tmp/m264/new10_time.txt). They were left in the main pass
# rather than carved out: carving another giant was declined on
# 2026-09-24, with the CI ceiling raised to 150 minutes instead.
#
# Refreshed again with the floors, from /tmp/m265/p2/on.txt. The off
# side of the same binary printed the numbers of m264 above to the
# digit (/tmp/m265/p2/off.txt), so the series did 0.35% more classes,
# 0.68% more expansions, 1.26% more bodies and three more points: nine
# models built whole that do not run yet. The flattening time per
# model read 4162ms off and 3957ms on, over the same 1037 models from
# the same binary run one after the other; the fall was not traced and
# is taken for weather, not for a saving.
#
# Refreshed again on 2026-09-24 for a carving and not for a change of
# the compiler: the three ReferenceAir moist-air examples left for the
# scheduled run (see the head of this file), which reverses the
# "declined" above - Roman decided the other way the same day. Built
# whole they were a quarter of the flatten half, and the counts fell
# with them far enough to leave the band for no fault: from
# /tmp/m266/main.txt, 283282 classes (0.04% fewer), 92505304
# expansions (0.96% fewer), 1337023 bodies (5.19% fewer - outside the
# band), 32249779 points and 44495032 Newton steps (49 and 60 fewer,
# the one of the three that ran).
#
# And the run counts refreshed from /tmp/m266/on.txt, where the five
# Wagner models run: 741 more points and 31 more Newton steps than the
# off side of the same binary, the flattening counts identical to the
# digit.
#
# Refreshed from /tmp/m267/on2.txt, the off side of the same binary
# (/tmp/m267/off2.txt) printing the numbers above to the digit. The
# expansions rose 5.83% and left the band, the bodies 4.13%: that is
# the three models the series built whole for the first time, which
# alone count 6032970 expansions and 58743 bodies on the on side and
# 551996 and 5890 where they refused before (both measured over just
# those three with --only-from). Their 5480974 more expansions are
# more than the whole rise of 5396078, the other 1031 models doing
# 84896 fewer between them (0.09%); that difference was not traced.
# The two bends that now run add 102 points and 220 Newton steps.
#
# Refreshed from /tmp/m268/on.txt, the off side of the same binary
# (/tmp/m268/off.txt) printing the numbers above to the digit. The
# classes did not move; the expansions fell 3.58% and the bodies rose
# 0.78%, both inside the band. Those two were not traced model by
# model. The names looked up, printed and not held, rose 18% (1509 to
# 1781 million), so a later rise in the flatten half's time is to be
# looked for here first. The one model that runs now adds 46 points and
# 50 Newton steps.
WORK_CLASSES=283567
WORK_EXPANSIONS=94392961
WORK_BODIES=1402985
WORK_POINTS=32250668
WORK_NEWTON=44495333
WORK_JACOBIANS=324
WORK_PERCENT=5

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
echo "$report" | grep -E '^(classes:|runnable examples|time:|work:)'

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

# The work. Each count is read off the `work:` line by the word that
# follows it, and a count that is not there is a failure for the same
# reason a missing time is.
work_line="$(echo "$report" | grep '^work:' || true)"
work_of() {
  echo "$work_line" | sed -n "s/.* \([0-9][0-9]*\) $1[;,].*/\1/p; s/.* \([0-9][0-9]*\) $1\$/\1/p" | head -n 1
}
held() {
  local what="$1" now="$2" written="$3"
  if [ -z "$now" ]; then
    echo "WORK: the report did not say how many $what; the work line changed shape"
    status=1
    return
  fi
  # Within WORK_PERCENT of the written number, in integers: now * 100
  # against written * (100 +- percent).
  if [ $((now * 100)) -gt $((written * (100 + WORK_PERCENT))) ] ||
    [ $((now * 100)) -lt $((written * (100 - WORK_PERCENT))) ]; then
    echo "WORK: $what is $now against $written written here ($(awk "BEGIN { printf \"%.3f\", $now / $written }")x), outside ${WORK_PERCENT}%"
    status=1
  fi
}
held "classes instantiated" "$(work_of classes)" "$WORK_CLASSES"
held "expansions" "$(work_of expansions)" "$WORK_EXPANSIONS"
held "bodies worked out" "$(work_of bodies)" "$WORK_BODIES"
held "points evaluated" "$(work_of points)" "$WORK_POINTS"
held "newton iterations" "$(work_of newton)" "$WORK_NEWTON"
held "jacobians" "$(work_of jacobians)" "$WORK_JACOBIANS"

if [ "$status" -eq 0 ]; then
  echo "OK: $read_now files read, $flatten_now flatten, $run_now run; runnable $runnable_flatten_now flatten, $runnable_run_now run"
  echo "OK: ${flatten_ms_now}ms per model flattening, ${run_ms_now}ms running"
fi
exit "$status"
