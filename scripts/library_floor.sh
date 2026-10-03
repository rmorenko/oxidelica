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
#
# And flatten 918 = 917 plus ModelicaTest.Utilities.TestReadFile, a
# runnable example, which reads a file three ways and now has the
# count of its lines, `readLine`'s end-of-file flag and `readFile`'s
# lines by place (/tmp/m270/off2.txt against /tmp/m270/off12.txt, one
# binary, OXIDELICA_NO_FILE_COUNTS the only difference). Nothing left
# the flatten list.
#
# And flatten 919 = 918 plus ModelicaTest.Math.TestMatrices2b, a
# runnable example, which takes the `dgesvd` chain of m272 whole: the
# decomposition, `dgetrf` and `dgetri` answered here, a shape handed in
# outranking an input's default, and a call standing as a statement
# given the body's bindings before its arguments are read
# (/tmp/m274/on.txt against /tmp/m274/off.txt, one binary, the off side
# with OXIDELICA_NO_SVD, OXIDELICA_DEFAULT_SHAPES_WIN and
# OXIDELICA_STATEMENT_ARGS_LATE set). Nothing left the flatten list.
#
# And flatten 928 = 919 plus nine tables read from files, from
# /tmp/m284/on.txt against /tmp/m284/off.txt (one binary,
# OXIDELICA_OLD_FILE_TABLES the only difference): CombiTable1Ds.Test35,
# CombiTable1Dv.Test35 and CombiTimeTable.Test89 read a comma-separated
# file, CombiTimeTable.Test80 and Test81 a field of a MATLAB structure,
# CombiTable2Ds.Test12 and CombiTable2Dv.Test12 a compressed version 7
# file, CombiTable2Ds.Test32 and CombiTable2Dv.Test32 a comma-separated
# grid. All nine are runnable examples and all nine run. Nothing left
# the flatten list.
#
# And flatten 936 is the runner's count of 563c39f (job 108413191618 of
# run 36245267382, /tmp/m287/runner.log), which covers af2249e as
# well: 928 plus the eight Noise examples of m286 whose walked body
# could not draw. The desk counts the same 936 on that tree
# (/tmp/m286/new.txt), so there is no lower of the two to take.
#
# And flatten 944 is the runner's count of 307a790 (job 108441560249 of
# run 36255558366, /tmp/m288/runner307.log), which covers 8c110ff and,
# ce66aca being prose alone, the whole m287 series: 936 plus the eight
# registers DFFREG, DFFREGL, DFFREGSRH, DFFREGSRL, DLATREG, DLATREGL,
# DLATREGSRH and DLATREGSRL, whose loop is left on a flag. The desk
# counts the same 944 on that tree (/tmp/m287/new.txt).
#
# And flatten 948 is the runner's count of d0a39b4 (job 108464947025 of
# run 36263962445, /tmp/m289/runner_d0a.log), which covers eed1ce0,
# 3bc838b and 06fc779, d0a39b4 being prose on top of them: 944 plus the
# three BackSample examples, whose clocks can never tick at one instant,
# and LinearColdWater, whose constant array is now read under the
# medium. The desk counts the same 948 on that tree
# (/tmp/m288/sub_new.txt).
#
# And flatten 957 is the runner's count of 0cabdad (job 108591424474 of
# run 36309091249, /tmp/m292/runner0cab.log): 948 plus the two CCCV
# stacks of the m290 series (eee4fe9, 82e6206) and the seven mixtures
# of 5a12a7b, whose field is read across a medium's array of records.
# The desk counts the same 957 on that tree (/tmp/m291/on.txt). The run
# lists of the two differ by the two names known to swing, Dimmer_RL on
# the runner and SpringWithMass on the desk, and are 658 on both.
#
# And flatten 959 is the runner's count of 61a974c (job 108616432147 of
# run 36318060838, /tmp/m293/runner61a.log), which carries the m292
# series of 9d0a598: 957 plus BatchPlant_StandardWater and
# TestWaterPumpNPSH, whose modifier calls are now read under the
# writer's medium. The desk counts the same 959 on that tree
# (/tmp/m292/e/on.txt, /tmp/m293/p/on.txt). The runner's log names only
# the models that run, so the flatten lists were compared by count.
#
# And flatten 961 is the runner's count of 50b0413 (job 108722878063 of
# run 36355702803, /tmp/m297/runner.log): 959 plus
# ModelicaTest.Math.Random.TestDistributions and
# TestTruncatedDistributions, whose `derTwoSided` answers `Y[size(X,
# 1)]` and is now taken at the length the call hands in. Both flatten
# and neither runs: their run wall is `unknown function linspace`. The
# desk counts the same 961 on that tree (/tmp/m296/q/on.txt), and run
# stays 665 on both. The run lists differ by the two names known to
# swing, Dimmer_RL on the runner and SpringWithMass on the desk.
#
# And flatten 963 = 961 plus ModelicaTest.Tables.CombiTimeTable.Test87
# and Test88, whose array handed down an extends clause is now cut into
# its elements (f999f45). The runner counts 963 in job 109136903901 of
# run 36484179633 (/tmp/m306/runner_job.log), the desk 963 on the same
# code (/tmp/m305/on.txt).
#
# And flatten 962 = 963 less Modelica.Electrical.Spice3.Examples.Oscillator,
# carved into `scripts/heavy_models.txt` on 2026-09-30. It runs since
# the zero-parameter quench took `i*R` with `R = 0` (e45a2aa), and it
# costs 233s and 456922 Newton steps alone: a solver benchmark of the
# adders' class. This is a deepening by the carving rule, not a loss:
#
#   flatten          963 = 962 here + 1 in the scheduled run
#   runnable flatten 847 = 846 here + 1 in the scheduled run
#   run 681 and runnable run 639 are unmoved: the Oscillator did not
#   run on the tree those floors were set from.
#
# Main numbers from one binary over the corpus (/tmp/m318/p_new.txt,
# 963/683 and 847/641 with the Oscillator still in), the scheduled
# half from the same binary (/tmp/m318/heavy.txt, 10/2 and 8/2).
#
# And flatten 966 = 962 plus the four of the impure generator, which
# hold its state in the model rather than in C since 7ba42fa5961357cefd4efcd27be59702a71dec10:
# ImpureGenerator, TestRandomIntegers, TestRandomNumbers and the block
# Noise.Utilities.ImpureRandom. The runner printed 966 / 685 and
# 849 / 643 on 7ba42fa5961357cefd4efcd27be59702a71dec10, in job 110171228157 of run 36799759552
# (/tmp/m325/ci_7ba42fa.log), the desk the same four numbers with the
# binary of m324 (/tmp/m324/corpus_new.txt:140-141). Set from the
# runner, which is not above the desk on any of them.
FLATTEN_FLOOR=966
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
#
# And run 591 = 590 plus TestReadFile, set out at `FLATTEN_FLOOR`,
# which runs as well. Nothing left the run list.
#
# And run 592 = 591 plus QuadratureLobatto3, whose integrand is handed
# on to `quadStep` rather than called, and is now specialized one call
# deeper (/tmp/m272/on.txt against /tmp/m272/off.txt, one binary). The
# run list gained exactly that name and lost none; the flatten list did
# not move.
#
# And run 593 = 592 plus TestMatrices2b, set out at `FLATTEN_FLOOR`,
# which runs and whose own comparisons hold. The run list gained that
# name and lost none.
#
# And run 594 = 593 plus
# Modelica.Mechanics.MultiBody.Examples.Elementary.Surfaces, a runnable
# example, whose colour map `ColorMaps.jet` sizes a local array by a
# single local worked out before it, which is now handed to the arrays
# as well (/tmp/m276/on.txt against /tmp/m276/off.txt, one binary, the
# off side with OXIDELICA_NO_SCALAR_HANDED). The run list gained that
# name and lost none, and the flatten list did not move.
#
# And run 595 = 594 plus
# Modelica.Media.Examples.TwoPhaseWater.TestTwoPhaseStates, a runnable
# example, written inside a package that extends the water medium and
# now read under that package, so `ph_explicit` is the medium's `true`
# and not a name nothing declares (/tmp/m276/on2.txt against
# /tmp/m276/off2.txt, one binary, the off side with
# OXIDELICA_NO_BARE_ENCLOSING). The run list gained that name and lost
# none, and the flatten list did not move.
#
# And run 596 = 595 plus
# Modelica.Magnetic.QuasiStatic.FundamentalWave.Examples.BasicMachines.InductionMachines.IMS_Start,
# a runnable example, whose rotor power is handed through a
# redeclaration two levels down and now reaches its body with the
# length of `vr` rather than the number of fields of one `Complex`
# (/tmp/m277/on.txt against /tmp/m277/off.txt, one binary, the off
# side with OXIDELICA_NO_WRITERS_LENGTHS_BELOW). The run list gained
# that name and lost none, and the flatten list did not move.
#
# And run 606 is the runner's count of ec6c163 (job 108096607540 of
# run 36142840775): its 595 of the red job of c5a80bc, plus the eleven
# models that tree brought in - the eight of the fixed-start column,
# ArmatureStroke, IMC_Characteristics, IMS_Characteristics, HeatLosses,
# SimpleLiquidWater, TestCylinder, MixingUnitWithContinuousControl and
# ComparisonPullInStroke, and the three of the inner loudness,
# SMPM_Braking, Polyphase.Rectifier and IMC_DOL. The desk counts 607
# on the same tree (/tmp/m278/k_on.txt), and the two run lists differ
# by exactly three names: only the desk runs DrumBoiler and
# SpringWithMass, only the runner runs Dimmer_RL. The floor is taken
# from the lower machine. Flatten 919 is the same on both.
#
# And run 614 is the runner's count of 9e6f772 (job 108171900686 of
# run 36165430620): 606 above plus the eight m279 won before
# `firstTick` - IdealGasN2, HoldWithDAeffects1, HoldWithDAeffects2,
# TickBasedPulse, TickBasedSine, UniformNoise, SampleWithADeffects and
# ClockedWith_AD_DA_Effects - each of them in the runner's run list.
# The desk counts 615 on the same tree (/tmp/m279/on4.txt), and the two
# lists differ by the same three names as before: only the desk runs
# DrumBoiler and SpringWithMass, only the runner runs Dimmer_RL.
#
# And run 615 is the runner's count of 385dce3 (job 108182204408 of
# run 36168556508): 614 plus RotationalSample, the one name its run
# list gained over 9e6f772's. The desk counts 616 on that tree
# (/tmp/m279/on5.txt) by the same three swinging names.
# MomentumBalanceFittings came after it, with 01e0b09, and is not in
# this floor: the runner has not yet counted a tree that holds it.
#
# And run 616 is the runner's count of b87de9e (job 108229171421 of
# run 36182860646): 615 plus MomentumBalanceFittings. The desk counts
# 617 on that tree (/tmp/m280/on2.txt) by the same three swinging
# names: only the desk runs DrumBoiler and SpringWithMass, only the
# runner runs Dimmer_RL.
#
# And run 627 is the runner's count of 72ad638 (job 108267726337 of
# run 36194686934): 616 plus the ten m281 won - five pump Machines,
# TestTemperature2, TestInitialization, BranchingPipes15,
# BranchingPipes16 and LinearColdWater - plus DrumBoiler, which the
# runner now runs as the desk always did. The desk counts 627 on the
# same tree (/tmp/m281/on2.txt), and the two run lists differ by two
# names: only the desk runs SpringWithMass, only the runner runs
# Dimmer_RL (/tmp/m282/d_ran.txt against r_ran.txt).
#
# And run 629 is the runner's count of 473a9bb (job 108304685526 of
# run 36206714158, /tmp/m284/runner.log): 627 plus the two m282 won,
# TestWaterPumpDefault and InverseParameterization. The desk counts
# 629 on the same code (/tmp/m282/on4.txt), and the two run lists
# differ by the same two swinging names and no other: only the desk
# runs SpringWithMass, only the runner runs Dimmer_RL
# (/tmp/m284/d_ran.txt against r_ran.txt).
#
# And run 638 = 629 plus the nine tables set out at `FLATTEN_FLOOR`,
# each of them in the run list of /tmp/m284/on.txt and none in that of
# /tmp/m284/off.txt; nothing left the run list. They are the desk's
# count, and the runner has not counted them yet: they read files and
# no solver is in their way, so nothing swings between the machines.
#
# And run 643 is the runner's count of 563c39f (job 108413191618 of
# run 36245267382): 638 plus AutomaticSeed, Distributions,
# DrydenContinuousTurbulence, UniformNoise and
# UniformNoiseXorshift64star, the five of m286 that run. The desk
# counts 643 on the same tree (/tmp/m286/new.txt).
#
# And run 651 is the runner's count of 307a790 (job 108441560249):
# 643 plus the eight registers set out at `FLATTEN_FLOOR`, all of which
# run. The desk counts 651 on the same tree (/tmp/m287/new.txt).
#
# And run 655 is the runner's count of d0a39b4 (job 108464947025): 651
# plus the four set out at `FLATTEN_FLOOR`, all of which run. The desk
# counts 655 on the same tree (/tmp/m288/sub_new.txt).
#
# And run 658 is the runner's count of 8ce586b (job 108489002796 of run
# 36272505121), which carries the code of e3b8651: 655 plus
# TestCheckValve, ReferenceAir_pT and ReferenceAir_ph, the three of
# m289 that run. The desk counts 658 on the same tree
# (/tmp/m289/on.txt), and flatten stays 948 on both.
#
# And run 662 is the runner's count of 61a974c (job 108616432147,
# /tmp/m293/runner61a.log): 658 plus TestWaterPumpNPSH,
# AST_BatchPlant.Test.TankWithEmptyingPipe1, AST_BatchPlant.Test.TwoTanks
# and Polyphase.Examples.TestSensors, the four of the m292 series that
# run. The desk counts 662 (/tmp/m293/p/on.txt); the two lists differ
# by Dimmer_RL on the runner and SpringWithMass on the desk, as before.
#
# And run 665 is the runner's count of 82f6834, which carries the code
# of b910fdd (job 108663101910 of run 36334661929,
# /tmp/m295/runner8a0.log): 662 plus TestOnly.FlueGas, IdealGasN2Mix
# and MixIdealGasAir, the three mixtures that were handed a record's
# text in a walked body. The desk counts 665 on the same code
# (/tmp/m295/p/ox1.txt), and flatten stays 959 on both.
#
# And run 667 is the lower of two counts of the linspace series of
# 2599b60. The runner's count of 21dc0aa (job 108790150900 of run
# 36378836704, /tmp/m299/ci_lib_full.log) is 668: 665 plus
# ModelicaTest.Math.Random.TestSpecial, TestDistributions and
# TestTruncatedDistributions, whose wall was `unknown function
# linspace`. The desk's pair of 86c8d93 (/tmp/m299/on2.txt, off2.txt)
# counts 667 on both halves: the same three, with SpringWithMass
# refused at its algebraic loop while two passes shared the machine,
# though it runs on its own. The runner has Dimmer_RL and not
# SpringWithMass, as before. Flatten stays 961 on both.
#
# And run 668 is where the runner and the desk now agree. 667 was the
# desk's lower count while two passes shared the machine; the desk now
# counts 668 on both halves of the pair of e44cd03 (/tmp/m302/on.txt,
# off.txt), and the runner counts 668 in the scheduled job
# 108946559041 of run 36427973736 on 3a4b9a6
# (/tmp/m303/runner_job.log). The two lists differ by the swing alone:
# Dimmer_RL on the runner, SpringWithMass on the desk. Flatten stays
# 961 on both.
#
# And run 671 = 668 plus InverseIncompressible_sh_T,
# TestValvesIncompressibleReverse and IncompleteMedia.Glycol47, the
# three that the fold and the slice of 5f435c3 brought to run. The
# runner counts 671 in job 109072163529 of run 36464829405 on 5f435c3
# (/tmp/m305/runner_job.log), and the desk 671 on the pair of the same
# code (/tmp/m303/on.txt). The two lists differ by the swing alone:
# Dimmer_RL on the runner, SpringWithMass on the desk. Flatten stays
# 961 on both.
#
# And run 673 = 671 plus the same Test87 and Test88, both of which
# run: 673 on the runner in job 109136903901 (/tmp/m306/runner_job.log)
# and 673 on the desk (/tmp/m305/on.txt), the lists differing by the
# swing alone, Dimmer_RL on the runner and SpringWithMass on the desk.
#
# And run 675 = 673 plus Modelica.Blocks.Examples.Noise.
# NormalNoiseProperties and UniformNoiseProperties, which fell to a
# step size underflow chasing `time >= t_0 + 1e-7` and run now that the
# threshold is scheduled as a time event: 675 on the runner in job
# 109207542783 of run 36506020824 on b7f04a3 (/tmp/m307/runner_raw.log)
# and 675 on the desk (/tmp/m307/pfq.txt), the lists differing by the
# swing alone, Dimmer_RL on the runner and SpringWithMass on the desk.
#
# And run 676 = 675 plus Modelica.Mechanics.MultiBody.Examples.
# Elementary.ForceAndTorque, which runs now that a component is built
# before the declaration above it that reads its members: 676 on the
# runner in job 109306337264 of run 36537969995 on 5ad730f
# (/tmp/m309/runner_job.log) and 676 on the desk (/tmp/m308/on3.txt),
# the lists differing by the swing alone, Dimmer_RL on the runner and
# SpringWithMass on the desk.
#
# And run 679 = 676 plus ModelicaTest.Media.TestOnly.R134a_setState_phX,
# R134a_setState_pTX and R134a_setState_pTX_high_T, which run since
# d81de04 gave a walked body's local records the values their
# declarations write: 679 on the runner in job 109491191619 of run
# 36593157901 on ea2e93e (/tmp/m312/ci_ea2.log), the same list as run
# 36593103106 on d81de04 (/tmp/m312/ci_d81.log), and 679 on the desk
# (/tmp/m312/on.txt), the lists differing by the swing alone, Dimmer_RL
# on the runner and SpringWithMass on the desk. Raised to what the
# runner printed rather than ahead of it, which is what ea2e93e undid.
#
# And run 680 = 679 less the swing plus AsymmetricalLoad and
# GearConstraint, which run since 02121e2 differentiated another member
# of the singular subset. The runner printed 681 in job 109671097541 of
# run 36646659434 on a0f96f1, code identical to 02121e2
# (/tmp/m316/ci_a0f96f1.log); the desk printed 680 with one binary
# built from the same code (/tmp/m316/old.txt, the old half of the m316
# pair). The lists differ by the swing alone: the runner has Dimmer_RL,
# the desk has neither Dimmer_RL nor SpringWithMass. Set from the lower.
#
# And run 681 = 680 plus ReferenceAir_dT, which runs since 00f3cd2 left
# a call too deep to read again under the flat model's name. The runner
# printed 682 in job 109725113083 of run 36664150519 on 5e0d945
# (/tmp/m317/ci_5e0d945.log, red on the time ceiling alone); the desk
# printed 681 on the same code without either swinging model
# (/tmp/m316/new.txt) and 682 with SpringWithMass (/tmp/m317/p_old.txt).
# Set from the lower.
#
# And run 682, a floor catching up with the runner rather than a model
# won here. The runner printed 682 on f76e71b (/tmp/m318/ci_f76e71b.log)
# and 962 / 682 again on d064889, in job 109821528831 of run 36695245820
# (/tmp/m319/ci_d064889.log:1283), and the two lists of models run are
# identical name for name. The desk printed 962 / 682 with the binary of
# m319 under its old key (/tmp/m319/p_old.txt). The totals agree while
# the swing still sits between them: the runner has Dimmer_RL and not
# SpringWithMass, the desk the other way round.
#
# And run 685 = 682 plus ImpureGenerator, TestRandomIntegers and
# TestRandomNumbers, by the same runner job as `FLATTEN_FLOOR`. The
# lists of models run on the runner and on the desk are 685 each and
# differ by the swing alone, Dimmer_RL on the runner and SpringWithMass
# on the desk, so the lower of the two is the same number.
#
# And run 686, a floor catching up with the runner after the series of
# the `pre` fix, the ladder and the crossing guard (m331). The runner
# printed 966 / 686 and runnable 849 / 644 on 62ebd6b, in job
# 110631858208 of run 36940891022 (/tmp/m332/ci_62ebd6b.log:1309-1310);
# the desk printed 966 / 687 and 849 / 645 with the series in
# (/tmp/m331/on.txt). The two lists of models run differ by one name,
# SpringWithMass, run on the desk and not on the runner
# (/tmp/m332/ran_runner.txt against /tmp/m332/ran_desk_on.txt). Set
# from the lower: 686 = 685 + RLV_Characteristic.
#
# And run 687 = 686 + TestJunctionVolume, the merge of 18da348. The
# runner printed 966 / 687 and runnable 849 / 645 on 0598f2b, in job
# 111268548956 of run 37145520603 (/tmp/m346/runner.log); the desk
# printed 966 / 689 and 849 / 647 on the same tree
# (/tmp/m345/pair/k.txt:1203-1204). The two lists of models run differ
# by the two known swings and nothing else, SpringWithMass and IMC_DOL,
# run on the desk and not on the runner (/tmp/m346/runner_ran_0598f2b.txt
# against /tmp/m346/desk_ran.txt). Set from the lower.
RUN_FLOOR=687
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
#
# And runnable flatten 803 = 802, runnable run 549 = 548, both plus
# TestReadFile set out at `FLATTEN_FLOOR` (/tmp/m270/off2.txt).
#
# And runnable run 550 = 549 plus QuadratureLobatto3, a runnable
# example, set out at `RUN_FLOOR` (/tmp/m272/on.txt).
#
# And runnable flatten 804 = 803, runnable run 551 = 550, both plus
# TestMatrices2b set out at `FLATTEN_FLOOR` (/tmp/m274/on.txt).
#
# And runnable run 552 = 551 plus Surfaces, set out at `RUN_FLOOR`
# (/tmp/m276/on.txt).
#
# And runnable run 553 = 552 plus TestTwoPhaseStates, set out at
# `RUN_FLOOR` (/tmp/m276/on2.txt).
#
# And runnable run 554 = 553 plus the quasi-static IMS_Start, set out
# at `RUN_FLOOR` (/tmp/m277/on.txt).
#
# And runnable run 564 is the runner's count of ec6c163: its 553 of
# the red job of c5a80bc plus the eleven set out at `RUN_FLOOR`, all
# runnable examples. The desk counts 565, by the same three names;
# runnable flatten is 804 on both.
#
# And runnable run 572 is the runner's count of 9e6f772: 564 plus the
# same eight set out at `RUN_FLOOR`, all runnable examples. The desk
# counts 573 on that tree; runnable flatten is 804 on both.
#
# And runnable run 573 is the runner's count of 385dce3: 572 plus
# RotationalSample, a runnable example.
#
# And runnable run 574 is the runner's count of b87de9e (run
# 36182860646): 573 plus MomentumBalanceFittings, a runnable example.
# The desk counts 575 on that tree; runnable flatten is 804 on both.
#
# And runnable run 585 is the runner's count of 72ad638 (run
# 36194686934), the same as the desk's on that tree
# (/tmp/m281/on2.txt): the eleven set out at `RUN_FLOOR` are all
# runnable examples, and the two swinging names cancel. Runnable
# flatten is 804 on both.
#
# And runnable run 587 is the runner's count of 473a9bb (job
# 108304685526 of run 36206714158), the same as the desk's
# (/tmp/m282/on4.txt): the two m282 won are runnable examples.
# Runnable flatten is 804 on both.
#
# And runnable flatten 813 and runnable run 596 are both nine above,
# the tables set out at `FLATTEN_FLOOR` (/tmp/m284/on.txt).
#
# And runnable flatten 820 = 813 plus seven of the eight Noise examples
# set out at `FLATTEN_FLOOR`, runnable run 601 = 596 plus the five set
# out at `RUN_FLOOR`: the runner's count of 563c39f, the same as the
# desk's on that tree (/tmp/m286/new.txt).
#
# And runnable flatten 828 = 820 plus the eight registers, runnable run
# 609 = 601 plus the same eight: the runner's count of 307a790, the
# same as the desk's (/tmp/m287/new.txt).
#
# And runnable flatten 832 = 828 plus the four set out at
# `FLATTEN_FLOOR`, runnable run 613 = 609 plus the same four: the
# runner's count of d0a39b4, the same as the desk's
# (/tmp/m288/sub_new.txt).
#
# And runnable run 616 = 613 plus the same three set out at
# `RUN_FLOOR`, all runnable examples; runnable flatten stays 832. The
# runner's count of 8ce586b, the same as the desk's (/tmp/m289/on.txt).
#
# And runnable flatten 841 = 832 plus the same two stacks and seven
# mixtures set out at `FLATTEN_FLOOR`, all runnable examples: the
# runner's count of 0cabdad (/tmp/m292/runner0cab.log), the same as
# the desk's (/tmp/m291/on.txt). Runnable run stays 616 on both.
#
# And runnable flatten 843 = 841 plus BatchPlant_StandardWater and
# TestWaterPumpNPSH, runnable run 620 = 616 plus the four set out at
# `RUN_FLOOR`, all runnable examples: the runner's count of 61a974c
# (/tmp/m293/runner61a.log), the same as the desk's (/tmp/m293/p/on.txt).
#
# And runnable run 623 = 620 plus the same three set out at
# `RUN_FLOOR`, all runnable examples; runnable flatten stays 843. The
# runner's count of 82f6834 (/tmp/m295/runner8a0.log), the same as the
# desk's (/tmp/m295/p/ox1.txt).
#
# And runnable flatten 845 = 843 plus TestDistributions and
# TestTruncatedDistributions, both runnable examples set out at
# `FLATTEN_FLOOR`; runnable run stays 623. The runner's count of
# 50b0413 (/tmp/m297/runner.log), the same as the desk's
# (/tmp/m296/pffull.txt).
#
# And runnable run 625 = 623 plus the three set out at `RUN_FLOOR` and
# less SpringWithMass, the lower of the runner's 626
# (/tmp/m299/ci_lib_full.log) and the desk's 625 (/tmp/m299/on2.txt);
# runnable flatten stays 845 on both.
#
# And runnable run 626 is the same agreement as run 668 above: the
# runner's 626 in job 108946559041 (/tmp/m303/runner_job.log) and the
# desk's 626 on both halves of /tmp/m302; runnable flatten stays 845.
#
# And runnable run 629 = 626 plus the same three set out at
# `RUN_FLOOR`, all runnable examples: the runner's 629 in job
# 109072163529 (/tmp/m305/runner_job.log) and the desk's 629
# (/tmp/m303/on.txt); runnable flatten stays 845.
#
# And runnable 847 and 631 = 845 and 629 plus Test87 and Test88, both
# runnable examples: the runner's 847/631 in job 109136903901
# (/tmp/m306/runner_job.log) and the desk's 847/631 (/tmp/m305/on.txt).
#
# And runnable run 633 = 631 plus the same two Noise models, both
# runnable examples: the runner's 847/633 in job 109207542783
# (/tmp/m307/runner_raw.log) and the desk's 847/633 (/tmp/m307/pfq.txt).
#
# And runnable run 634 = 633 plus ForceAndTorque, a runnable example:
# the runner's 847/634 in job 109306337264 (/tmp/m309/runner_job.log)
# and the desk's 847/634 (/tmp/m308/on3.txt).
#
# And runnable run 637 = 634 plus the same three R134a models, all
# runnable examples: the runner's 847/637 in job 109491191619
# (/tmp/m312/ci_ea2.log) and the desk's 847/637 (/tmp/m312/on.txt).
#
# And runnable run 638 by the same pair: the runner's 847/639 and the
# desk's 847/638, set from the lower for the same swing.
#
# And runnable run 639 = 638 plus ReferenceAir_dT, a runnable example,
# by the same three: the runner's 847/640 on 5e0d945, the desk's 847/639
# without the swing and 847/640 with it. Set from the lower.
#
# And runnable flatten 846 = 847 less the Oscillator, a runnable
# example carved into the scheduled run; see `FLATTEN_FLOOR`.
#
# And runnable run 640 by the same runner job as `RUN_FLOOR` above: the
# runner printed 846 / 640 on d064889 (/tmp/m319/ci_d064889.log:1284) and
# the desk 846 / 640 (/tmp/m319/p_old.txt).
#
# And runnable 849 / 643, the three runnable examples of the impure
# generator, by the runner job of `FLATTEN_FLOOR` on 7ba42fa5961357cefd4efcd27be59702a71dec10; the
# helper block is not runnable and moves only the flatten count.
#
# And runnable run 644 by the same runner job as `RUN_FLOOR` (62ebd6b,
# job 110631858208): 849 / 644 there against 849 / 645 on the desk, the
# difference SpringWithMass. Flatten 849 stands, the runner printing no
# more.
#
# And runnable run 645 by the same runner job as `RUN_FLOOR` (0598f2b,
# job 111268548956): 849 / 645 there against 849 / 647 on the desk, the
# difference the same two swings.
RUNNABLE_FLATTEN_FLOOR=849
RUNNABLE_RUN_FLOOR=645
# Every file of the library parses. This is a ceiling reached rather
# than a floor to hold, so it is written as the number left over: one
# file that stops parsing takes its whole tree of classes with it, and
# the counts below would hide that behind a handful of models.
UNREAD_CEILING=0
# The time a model costs is held by the ratio of the two halves, and
# not by a ceiling on either. Both halves are measured in one pass, on
# one machine, under one weather, so the weather cancels out of their
# ratio and does not cancel out of either alone.
#
# The absolute ceilings of 12000ms per model that stood here sat inside
# the build machine's own band: over one and the same code it printed
# anything from 6622 to 12957ms per model running, and in fifteen hours
# three commits went red on counts that were right (02121e2 at 12957,
# 5e0d945 at 12185, f76e71b at 12578). A threshold inside the noise
# measures the noise.
#
# Over 46 library jobs the ratio of running to flattening came out
# between 0.832 and 1.295, median 1.045, and spread 7.9% where running
# alone spread 14.9%. The band below fired on none of the 46; it has
# 19% of room under the lowest and 16% over the highest, so it catches
# flattening growing dearer by 49% and running growing dearer by 44%.
# It is two-sided on purpose: a regression in flattening moves the
# ratio down, and a one-sided check would never see it.
#
# What it does not catch, said outright: both halves slowing together,
# which the ratio cancels exactly as it cancels the weather - the job's
# own time limit is what stops a catastrophe of that kind - and any
# regression smaller than about 44%, which this machine cannot tell
# from its weather at all. The milliseconds per model are still
# printed, for the eye and for the next measurement of drift; they are
# only no longer judged.
#
# The band is the runner's, and a desk runs higher: 45 desk passes
# (the list is kept beside the shift notes, desk_ratios_m319_dedup.tsv)
# gave ratios from 1.139 to 1.795, median 1.396, 95th percentile 1.717,
# so the runner's band turned a right desk pass red about one time in
# three. The environment may therefore move the edges, and the
# preflight does, to 0.97 and 2.10 - the same room of about 17% on
# either side that the runner's band leaves round the runner's spread.
# CI sets nothing and keeps 0.70..1.50.
RATIO_LOW="${RATIO_LOW:-0.70}"
RATIO_HIGH="${RATIO_HIGH:-1.50}"

# The band itself, as a function, so that it can be seen red without a
# library pass: `library_floor.sh --ratio-check <flatten ms> <run ms>`
# judges the two numbers it is given and nothing else. A check nobody
# has seen fail is a check nobody has checked.
ratio_verdict() {
  awk -v f="$1" -v r="$2" -v lo="$RATIO_LOW" -v hi="$RATIO_HIGH" 'BEGIN {
    if (f <= 0) { printf "CEILING: flattening cost %sms per model, and a ratio over it means nothing\n", f; exit 1 }
    x = r / f
    if (x < lo || x > hi) { printf "CEILING: run/flatten ratio is %.3f, and the band is %.2f..%.2f\n", x, lo, hi; exit 1 }
    printf "ratio: run/flatten is %.3f, inside %.2f..%.2f\n", x, lo, hi
  }'
}
if [ "${1:-}" = "--ratio-check" ]; then
  ratio_verdict "${2:?usage: library_floor.sh --ratio-check <flatten ms> <run ms>}" \
    "${3:?usage: library_floor.sh --ratio-check <flatten ms> <run ms>}"
  exit $?
fi

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
# 1.2 billion between the two passes, and was printed and not held on
# the grounds that a count that wanders cannot be a ratchet. That was
# the wrong comparison: the wander was set against zero and not against
# the signal. Every count wanders somewhere; what decides is how far
# below what it is there to catch. The names wander by about one part
# in 37 million and caught a rise of 18% that no held count saw (m268,
# below), about 4 600 000 to one; the expansions caught the same change
# at 3.6% against a band of five, about 44 to one. So the names are
# held too, under a band of their own measured from their own noise -
# see WORK_NAMES. A ratchet that fires for nothing is still one
# somebody turns off, which is why the band is set from a measurement
# and wide above it.
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
#
# Refreshed from /tmp/m270/off2.txt; its off side, /tmp/m270/off12.txt,
# printed 283567 / 94393401 / 1402995 / 32250670 / 44495461 from the
# same binary. The file counts add 983 expansions, 64 bodies and the
# six points of TestReadFile.
#
# Refreshed from /tmp/m274/on.txt after the `dgesvd` chain; its off
# side, /tmp/m274/off.txt, printed 283567 / 94394474 / 1403059 /
# 32250713 / 44495461 and 1780978093 names from the same binary. The
# expansions rose 7399 and the bodies fell 66, both far inside the
# band; the points rose six, TestMatrices2b's. The names fell 1630140,
# 915 per million, while TestMatrices2b alone counts 197201 of its own
# (`--only`): the rest is the other models looking up fewer, which
# was not traced model by model.
#
# Refreshed from /tmp/m292/e/on.txt, the m292 series; its off side,
# /tmp/m292/e/off.txt, printed 285496 / 95078516 / 1406275 / 32446799 /
# 44912614 from the same binary, the lists of m291 name for name. The
# expansions rose 11.1% and the bodies 9.9%, and one model owns them:
# BatchPlant_StandardWater counted 12074718 expansions and 159644
# bodies flattened against 2063004 and 23598 refused
# (/tmp/m292/bp_on.txt, bp_off.txt), 10011714 of the 10276799 and
# 136046 of the 139883. The four that now run add 252 points and 1350
# Newton steps.
WORK_CLASSES=287278
WORK_EXPANSIONS=105355315
WORK_BODIES=1546158
#
# Refreshed from /tmp/m328/new.txt, the warm start restored on a
# rejected step; its off side, /tmp/m328/old.txt
# (OXIDELICA_REJECT_KEEPS_GUESS, one binary /tmp/m328/ox), printed
# 32450181 points and 44916212 Newton steps. The fall of 4921899 points
# and 4736967 steps is Dimmer_RL, which burned 30415181 and 41888615
# before it was refused and spends 25482572 and 37132737 running
# (/tmp/m328/each_old.txt, each_new.txt, OXIDELICA_WORK_EACH).
#
# Split in two on 2026-10-01 (m330), the precedent of the Jacobians
# apart. Dimmer_RL is 92.6% of the corpus's points, runs on a desk and
# is refused on the build machine, and the band of five percent stood on
# which way it fell: the library job for 691b9e3 printed 33122763
# points and 47567421 Newton steps against 27528282 and 40179245 here,
# x1.2032 and x1.1839, red three runs running with every floor held
# (/tmp/m329/ci_691b9e3.log:1303-1307). Its run lists differ from the
# desk's by Dimmer_RL and SpringWithMass one way and RLV_Characteristic
# the other.
#
# So Dimmer_RL is held on lines of its own, and the band here is the
# corpus without it. What the two places are known to print for that:
#
#   desk     27528282 - 25482572 =  2045710 points
#            40179245 - 37132737 =  3046508 newton
#            (/tmp/m329/off.txt, /tmp/m328/each_new.txt)
#   machine  33122763 - 30415181 =  2707582 points
#            47567421 - 41888615 =  5678806 newton
#
# The machine's pair is a subtraction and not a print: 30415181 and
# 41888615 are what Dimmer_RL burned before its refusal on a desk
# (/tmp/m328/each_old.txt), and whether the build machine's refusal
# costs the same is not known. Read the other way, with the desk's rest,
# the machine's Dimmer_RL is 31077053 and 44520913. Both readings are
# covered below. The script now prints the dearest models by name, so
# the next log of the build machine says its own split, and the band is
# drawn tight on that print rather than on this subtraction.
#
# Drawn tight on 2026-10-01 (m331) on the build machine's own print.
# The library job for 1b8eadb (run 36904852421, the first under this
# split) named Dimmer_RL 31079574 points and 44524678 Newton steps, so
# the subtraction above guessed the machine's refusal 0.66 million
# points too cheap: the rest of the corpus is the same there as here.
# The two later runs, 40b5309 and 936f265, printed the same to the
# digit (/tmp/m331/lib_*.txt). What the places print for the rest:
#
#   machine  33122763 - 31079574 = 2043189 points
#            47567421 - 44524678 = 3042743 newton
#   desk     27528282 - 25482572 = 2045710 points    (main, m330)
#            40179245 - 37132737 = 3046508 newton
#   desk     25740111 - 23662990 = 2077121 points    (the m331 series
#            37426193 - 34340892 = 3085301 newton     of the pre fix,
#                                                     the ladder and
#                                                     the crossing)
#
# the last from /tmp/m331/on.txt, one binary whose other half
# /tmp/m331/off.txt repeats main's 27528282 and 40179245. The series
# adds 31411 points and 38558 Newton steps to the rest, named model by
# model in /tmp/m331/w_diff.txt: DemoPowerSupplyWithBuffer +21724,
# ComparisonPullInStroke +12777, ComparisonQuasiStatic -10773 among
# them. Centred on the three and held to five percent again:
#
#   points  2060155 +- 5%  =  1957147 .. 2163163
#           x0.992 .. x1.008
#   newton  3064022 +- 5%  =  2910821 .. 3217223
#           x0.993 .. x1.007
WORK_POINTS=2060155
WORK_POINTS_PPM=50000
WORK_NEWTON=3064022
WORK_NEWTON_PPM=50000
# And Dimmer_RL, whichever way it falls, centred between the desk's run
# under the series and the machine's printed refusal:
#
#   points  27371282 +- 20%  =  21897026 .. 32845538
#           desk run 23662990 x0.865 (main's 25482572 x0.931),
#           machine 31079574 x1.135
#   newton  39432785 +- 20%  =  31546228 .. 47319342
#           desk run 34340892 x0.871 (main's 37132737 x0.942),
#           machine 44524678 x1.129
#
# A refusal early in the run, or a run that doubles its work, fires.
WORK_APART_MODEL=Modelica.Electrical.PowerConverters.Examples.ACAC.Dimmer_RL
WORK_POINTS_APART=27371282
WORK_NEWTON_APART=39432785
WORK_APART_PPM=200000
# How many of the dearest models the log names on each count.
WORK_DEAREST=12
# Jacobians refreshed from /tmp/m278/k_on.txt, 605, after the m278
# series; the off side of one binary (/tmp/m278/off.txt, all three
# switches of the series set) printed 327. 275 of the 278 are named
# model by model (/tmp/m278/jac.txt, `--only` on each running model
# both ways), and the other three were not traced:
# ComparisonQuasiStatic 8 to 124 and ComparisonPullInStroke 0 to 150,
# both on the road that pairs a demoted coil current with its flux, so
# the currents start at 1e-12 where they started at 1e-21 and BDF
# takes 5642 points where it took 3105 - the same answer to 4e-6, at a
# cost in one model; SMPM_Braking 0 to 7 and IMC_DOL 1 to 3, which run
# now. Points and Newton steps rose 0.6% and 0.9%, inside the band.
#
# Split in two on 2026-10-01 (m327). The two solenoids named in
# `scripts/loose_jacobians.txt` build a number of Jacobians that says
# which difference step the run happened to take: ComparisonQuasiStatic
# built 0, 124 and 2812 at steps of 5e-9, 1e-8 and 2e-8, and
# ComparisonPullInStroke 131 to 150 (/tmp/m327/p_*.out). Summed with the
# rest they owned the band. `library check` now counts them apart, and
# one pass of one binary over the code of main (/tmp/m327b/old.txt)
# printed
#
#   597 = 323 here + 274 apart
#
# against 605 written before, the 597 being the sum the pair of m327
# printed for the same code (/tmp/m327/old.txt). The 274 is the two at
# the default step, 124 and 150, as measured one at a time.
#
# Moved by the warm start restored on a rejected step (m328), the pair
# /tmp/m328/old.txt and new.txt of one binary:
#
#   323 here + 274 apart  ->  130 here + 733 apart
#
# Apart, ComparisonQuasiStatic went 124 to 615 and ComparisonPullInStroke
# 150 to 118 (/tmp/m328/each_*.txt), 274 to 733. Here the fall of 193
# is named model by model (/tmp/m328/w_*.txt, OXIDELICA_WORK_EACH over
# the union of both run lists): ControlledSwitchWithArc 100 to 32,
# Dimmer_RL 146 to 13, IMC_Steinmetz 0 to 6 (runs now), SMPM_Braking
# 7 to 9, and three thyristor bridges 3 to 4, 5 to 6 and 7 to 5.
#
# Moved by the series of m331 (the `pre` fix, the ladder, the crossing
# guard), which was measured at 130 to 136 here and 733 to 600 apart on
# the desk (/tmp/m331/off.txt, on.txt) and drew the band of points and
# Newton steps but left this line where it was. The runner then printed
# 138 here and 600 apart on 62ebd6b (job 110631858208,
# /tmp/m332/ci_62ebd6b.log:1312) and turned red on this line alone, 1.06
# of 130. The rise of six on the desk is named model by model
# (OXIDELICA_WORK_EACH, /tmp/m332/jac_off.txt against jac_on.txt):
# Dimmer_RL 13 to 19, ThyristorBridge2mPulse_DC_Drive 5 to 9,
# Rectifier6pulse 4 to 5, Rectifier12pulse 8 to 7, IMC_Steinmetz 6 to 4,
# SMPM_Braking 9 to 7. The centre is the middle of the two readings of
# the same code, (136 + 138) / 2 = 137, and five percent around it is
# 130.15 to 143.85, which holds both.
WORK_JACOBIANS=137
# The pair is held on a line of its own and to a band of its own, an
# order of magnitude wide in either direction rather than five percent:
# 90% either side of 274 is 27 to 520, so a refusal (0) or the swing to
# 2812 fires, and a wander between the steps that both run does not.
# Around 733 (m328) the same width is 73 to 1393: a refusal still fires,
# and so does the swing to 2812.
WORK_JACOBIANS_APART=733
WORK_JACOBIANS_APART_PPM=900000
WORK_PERCENT=5
# The names looked up, held to a band of their own in parts per
# million. Measured on 2026-09-25 over one binary of 3103c80 and `.msl`
# under this checkout, four passes one after another:
# 1780978119, 1780978071, 1780978082, 1780978119 (/tmp/m273/on.txt,
# n2.txt, n3.txt, n4.txt). The widest spread is 48, 0.027 per million.
#
# That is not the noise that matters. The count depends on where the
# library stands on the disk, as the flattening counts do (see above),
# and far more than it wanders: the preflight's default library path
# printed 1781437975 (/tmp/m272/pf.txt, 258 per million above), and
# the build machine 1782419170 and 1782419122 for 78f754f and a03ad78
# (their library jobs, 809 per million above; that code sits 514
# names below this one on a desk, /tmp/m272/off.txt against on.txt).
# A band of a hundred times the run's wander, as first planned, would
# have been 2.7 per million and red on the build machine every time.
#
# So the band is 2000 per million: two and a half times the widest
# difference between two places, seventy-four thousand times the
# wander of one place, and still ninety times narrower than the 18%
# rise it is there to catch.
#
# Moved to 1779347953 from /tmp/m274/on.txt, the `dgesvd` chain; see
# the refresh above for the fall of 915 per million, which would have
# left the band less than half its width on the desk side.
#
# Moved to 1781489445 from /tmp/m284/off.txt, a desk pass over the
# code of 473a9bb. The reference had stood since m274 while the
# series after it each added a few hundred names per million, and the
# build machine's library job for 473a9bb (job 108304685526 of run
# 36206714158) printed 1782930267, 2013 per million above 1779347953
# and so red with every floor held. Against the new reference the
# build machine stands 809 per million above and the desk at zero.
#
# Moved to 1786972232 from /tmp/m287/new.txt, the m287 loop exit. One
# binary printed 1781541757 with OXIDELICA_NO_LOOP_EXIT set
# (/tmp/m287/old.txt) and 1786972232 without it, 3048 per million up, and
# the flatten lists differ by the eight DFF and DLAT registers alone:
# measured one at a time, the two DFFREGSR take 1.6 million names each,
# the two DLATREGSR 1.1 million and the four plain ones 0.2 million,
# where each had cost 0.12 million to refuse. That is the rise, bought
# with eight models, and the old reference would have been red on it.
#
# Moved to 1782427540 from /tmp/m288/sub_new.txt, the m288 series. Two
# of its three changes moved the count, each measured on one binary
# both ways: reading a medium's constant array under the medium first
# took 1786972150 to 1786595989 (/tmp/m288/arr_old.txt, arr_new.txt,
# 210 per million down, the flatten and run lists identical), and
# substituting a body's bindings once rather than twice took
# 1786595982 to 1782427540 (/tmp/m288/sub_old.txt, sub_new.txt, 2333
# per million down, with LinearColdWater added to both lists). Fewer
# names looked up for more models: the second pass was walking every
# name of every assignment's value again. Together 2543 per million
# below the old reference, which would have been red on the fall.
#
# Moved to 1907264675 from /tmp/m292/e/on.txt, the m292 series. One
# binary printed 1784082596 with its four keys set (/tmp/m292/e/off.txt,
# lists identical to m291 name for name) and 1907264675 without them,
# 69045 per million up. BatchPlant_StandardWater alone accounts for
# 119700548 of the 123182079: refused, it looked up 20293435 names, and
# flattened 139993983 (/tmp/m292/bp_off.txt, bp_on.txt). The rest is
# TestWaterPumpNPSH, which now runs. The rise is bought with a model
# that flattens, and the old reference would have been red on it.
#
# Moved to 1914710601 from /tmp/m298/q2/on.txt, the m298 pair. The
# guard of 006e952 asks every name of a body left standing twice, under
# the medium and without it, and the reference was not moved with it:
# one binary of m297 printed 1910204761 with the guard switched off
# (/tmp/m297/q/off.txt) and 1914710562 with it on (/tmp/m297/q/on.txt),
# 2359 per million up with the lists identical, and the build machine's
# library job for 006e952 (run 36366920031) printed 1914321460, 3700 per
# million above the old reference and so red with every floor held. The
# m298 walk adds 36 names on the desk (1914710565 off, 1914710601 on).
# Against the new reference the build machine for 006e952 stands 203
# per million below.
#
# Moved to 1916826359 from /tmp/m300/on.txt, step 1 of the gate series.
# One binary printed 1913669702 with `OXIDELICA_NO_CARRIED_MARK` set
# (/tmp/m300/off.txt) and 1916826359 without it, 1650 per million up,
# with the flatten and run lists identical both ways and the register
# identical line for line. The rise is the gate asking each body left
# standing under a medium whether that medium changes what it reads.
# The new reference is 1105 per million above the old one, inside the
# band, and is moved so the band is spent on what comes next rather
# than on this.
WORK_NAMES=1916826359
WORK_NAMES_PPM=2000

# The band, as a function, so that it can be seen red without a library
# pass: `library_floor.sh --work-check <now> <written> [<band ppm>]`
# judges the one count it is given against the one number and nothing
# else, the way `--ratio-check` does for the times.
status=0
held() {
  local what="$1" now="$2" written="$3"
  # A count may bring a band of its own, in parts per million, where
  # its noise is so far below a percent that a percent would catch
  # nothing. The rest go under WORK_PERCENT, which is 10000 per
  # million per point. The largest product is about 1.8e9 * 1e6, far
  # inside the shell's 9.2e18.
  local band="${4:-$((WORK_PERCENT * 10000))}"
  if [ -z "$now" ]; then
    echo "WORK: the report did not say how many $what; the work line changed shape"
    status=1
    return
  fi
  # Within the band of the written number, in integers: now * 1e6
  # against written * (1e6 +- band).
  if [ $((now * 1000000)) -gt $((written * (1000000 + band))) ] ||
    [ $((now * 1000000)) -lt $((written * (1000000 - band))) ]; then
    echo "WORK: $what is $now against $written written here ($(awk "BEGIN { printf \"%.6f\", $now / $written }")x), outside $band per million"
    status=1
  fi
}
if [ "${1:-}" = "--work-check" ]; then
  held "the count" "${2:?usage: library_floor.sh --work-check <now> <written> [<band ppm>]}" \
    "${3:?usage: library_floor.sh --work-check <now> <written> [<band ppm>]}" ${4:+"$4"}
  exit "$status"
fi

directory="${1:?usage: library_floor.sh <library directory>}"
cd "$(dirname "$0")/.."

# The names as well as the counts: the run half differs between one
# machine and another, and a difference nobody can name is a
# difference nobody can fix. The list goes to a file rather than the
# log, and the log gets the run half of it, which is where the
# machines disagree.
report="$(OXIDELICA_WORK_EACH=1 ./target/release/oxidelica library check --list --without scripts/heavy_models.txt "$directory")"
# The work of each model by name, taken out of the report and kept for
# the band below: a total that moved on the build machine and not on a
# desk said by how much and not who, and the build machine's log is the
# only place its own models can be named.
each_work="$(echo "$report" | grep '^  work  ' || true)"
report="$(echo "$report" | grep -v '^  work  ' || true)"
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
if [ -z "${flatten_ms_now:-}" ] || [ -z "${run_ms_now:-}" ]; then
  echo "CEILING: the report did not say what a model cost; the time line changed shape"
  status=1
else
  echo "time per model: ${flatten_ms_now}ms flattening, ${run_ms_now}ms running"
  ratio_verdict "$flatten_ms_now" "$run_ms_now" || status=1
fi

# The work. Each count is read off the `work:` line by the word that
# follows it, and a count that is not there is a failure for the same
# reason a missing time is.
work_line="$(echo "$report" | grep '^work:' || true)"
work_of() {
  echo "$work_line" | sed -n "s/.* \([0-9][0-9]*\) $1[;,].*/\1/p; s/.* \([0-9][0-9]*\) $1\$/\1/p" | head -n 1
}
held "classes instantiated" "$(work_of classes)" "$WORK_CLASSES"
held "expansions" "$(work_of expansions)" "$WORK_EXPANSIONS"
held "bodies worked out" "$(work_of bodies)" "$WORK_BODIES"
held "names looked up" "$(work_of names)" "$WORK_NAMES" "$WORK_NAMES_PPM"
# The dearest models by the run half's two counts, named in the log so
# that a band that fires on the build machine says which models it
# fired for without a desk having to guess by subtraction.
if [ -z "$each_work" ]; then
  echo "WORK: the report named no model's work; OXIDELICA_WORK_EACH changed shape"
  status=1
fi
# A measuring pipe is not cut short by `head` (see below), so the top
# is taken by `awk`, which reads its input to the end.
echo "dearest by points:"
echo "$each_work" | awk '{ print $3, $2 }' | sort -nr | awk -v n="$WORK_DEAREST" 'NR <= n { print "  " $0 }'
echo "dearest by newton:"
echo "$each_work" | awk '{ print $5, $2 }' | sort -nr | awk -v n="$WORK_DEAREST" 'NR <= n { print "  " $0 }'
# The model held apart, and the rest of the corpus without it.
apart_line="$(echo "$each_work" | awk -v m="$WORK_APART_MODEL" '$2 == m' || true)"
apart_points="$(echo "$apart_line" | awk '{ print $3 }')"
apart_newton="$(echo "$apart_line" | awk '{ print $5 }')"
if [ -z "$apart_points" ] || [ -z "$apart_newton" ]; then
  echo "WORK: the report did not say what $WORK_APART_MODEL cost"
  status=1
else
  echo "work apart: $WORK_APART_MODEL $apart_points points, $apart_newton newton"
  held "points evaluated apart" "$apart_points" "$WORK_POINTS_APART" "$WORK_APART_PPM"
  held "newton iterations apart" "$apart_newton" "$WORK_NEWTON_APART" "$WORK_APART_PPM"
  points_all="$(work_of points)"
  newton_all="$(work_of newton)"
  held "points evaluated" "${points_all:+$((points_all - apart_points))}" "$WORK_POINTS" "$WORK_POINTS_PPM"
  held "newton iterations" "${newton_all:+$((newton_all - apart_newton))}" "$WORK_NEWTON" "$WORK_NEWTON_PPM"
fi
held "jacobians" "$(work_of jacobians)" "$WORK_JACOBIANS"
held "jacobians counted apart" "$(work_of "jacobians apart")" "$WORK_JACOBIANS_APART" "$WORK_JACOBIANS_APART_PPM"

if [ "$status" -eq 0 ]; then
  echo "OK: $read_now files read, $flatten_now flatten, $run_now run; runnable $runnable_flatten_now flatten, $runnable_run_now run"
  echo "OK: ${flatten_ms_now}ms per model flattening, ${run_ms_now}ms running"
fi
exit "$status"
