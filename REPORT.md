# Shift 23: the mode-wise substitution, sieved and measured

## What was asked

Answer 22 in `QUESTION_FOR_FABLE.md` proposed variant 2: settle an `if`
*expression* whose condition holds still into the branch that holds, on
the reasoning that a conditional left standing gives the algebraic layer
a slope that is zero down one arm, and a block torn on such a slope
divides by that zero. The wall it was aimed at is the models whose
residual was never a number.

The instruction was to sieve first, write second, and measure third.
All three were done, and the third says the substitution does not pay.

## 1. The sieve

The wall is 29 models, not the 39 the census suggested: the census
counts *kinds* of refusal and `library check --refused` prints one line
per model, so several census rows share models. Measured with
`./target/release/oxidelica library check .msl --refused`, counting the
four kinds of run-time loop failure:

| kind | models |
| --- | --- |
| residual ... is NaN before any step | 21 |
| algebraic loop did not converge | 4 |
| singular Jacobian | 3 |
| algebraic loop diverged | 1 |
| **total** | **29** |

Each was then run under `OXIDELICA_MODE_PROBE=1`, which prints every
`if` inside a torn block together with the names its condition reads and
whether any of them is an unknown of that same block. Classified by the
answer:

| family | models | meaning |
| --- | --- | --- |
| A | 14 | every condition reads only parameters and discretes |
| AB | 8 | both kinds present in one model |
| B | 4 | at least one condition reads a continuous unknown of the block |
| none | 3 | no conditional in the block at all |

The prediction was that a third family would be found if there was one,
and there is: three models - `Resistor`, `DCSE_SinglePhase` and
`DC_CompareCharacteristics` - stand at the NaN wall with no `if` in the
guilty block. Their residual is not a number for some other reason
entirely, and no mode-wise anything will move them.

So the number the sieve gives, *before the code was written*, is: at
most 22 models (14 A plus 8 AB) could possibly be helped.

The pipe for every count above is
`./target/release/oxidelica library check .msl --refused` for the wall
and `--only <Class>` under the probe for the classification; the
per-model table is in `~/oxideflow/state/s23/sieve.txt`.

A note on the sieve's first run, because it nearly went into this
report as fact: it wrapped the probe in `timeout 120`, which does not
exist on macOS, and every one of the 29 models came back with zero
conditions found. Twenty-nine honest-looking zeros in a third of a
second. This is the failure `AGENTS.md` already describes under "a zero
counts only where the same pipe can print something other than zero",
and it was caught only because 29 identical zeros arriving that fast was
implausible, not because the pipe complained.

## 2. The small models

`ModeSwap`, the model answer 22 predicted would be red today, **is
green today** and numerically correct in both phases: it holds at 1
through the locked phase, flips at 0.5, and reaches `x(1) = e^0.5 =
1.648721`. The prediction is broken, and the reason matters - the
matching it was supposed to defeat is not fixed across the two modes in
the way the question assumed. `ZeroSlope` is red as predicted, with a
step size underflow at 0.5.

Several further small models were written to try to produce an honest
witness for the substitution, and all of them either ran on the base
compiler already or failed on it for an unrelated reason. The only
honest small witness found was a model with a conditional *coefficient*
whose else-arm is `0/0`, which fails at `t = 0` on the base compiler
and at the flip with the substitution in place.

## 3. The change, and why it is parked

The substitution was written: it walks the continuous equations after
`settle_modes`, replaces every `Expr::If` whose condition it will settle
with the arm that holds, folds the result so that an exposed `0 * z`
stops naming `z` to the matcher, and reports each pair into
`mode_conditions` so the run recompiles at the flip.

Two things were learned building it, and both are recorded because
neither is obvious:

The decision must be made against the environment `settle_modes`
settles, not against `start_env`. A discrete defined by an ordinary
equation - `locked = time < 0.5` - has only its declared start in the
raw environment, so deciding on it answers for a world half a step
behind. The symptom was a first row reporting `locked = 1` while the
sliding arm had been substituted, and it was caught by reading the first
row rather than the last.

The substitution is only half done until the arithmetic it exposes is
folded. A branch that becomes `0 * z` still *names* `z`, the matcher
pairs the equation with it, and the division by zero happens exactly as
before.

Then the corpus, from one binary each time:

| gate | flatten | run | runnable flatten | runnable run |
| --- | --- | --- | --- | --- |
| baseline (s22) | 820 | 386 | 722 | 381 |
| parameters and discretes | 820 | 364 | 722 | 359 |
| parameters only | 820 | 386 | 722 | 381 |

The wide gate costs 22 models net: 26 lost against 4 won. The run-list
diff names them, and they are one family - `CharacteristicIdealDiodes`
and eighteen bridges of `Modelica.Electrical.PowerConverters`. The
cause is that a discrete looks like it belongs in the gate and does
not. A diode's `off` is discrete and does hold still *between* events,
but it is precisely the discrete the event iteration is in the act of
deciding: settling on its present value hands the block a plan for the
mode it is leaving. What holds still between events and what holds
still across the event being resolved are different properties, and
only a parameter has both.

Narrowing the gate to parameters and constants removes every victim -
and every win with them. The run list is identical to the baseline line
for line, all five floors unmoved. That is the measurement that decides
it: the narrow gate is a change that does nothing, and the wide gate is
a change that costs 22 models to win 4.

The four the wide gate won are worth naming, because they are the whole
case for the idea and it is not enough: `CoupledClutches`, `Friction`,
`Dimmer_RL` and `ThyristorBridge2mPulse_RLV`.

So variant 2 is parked rather than taken. The patch is kept at
`~/oxideflow/state/s23/mode_substitution.patch` so that nobody has to
build it again; what is committed is the probe, which is what made the
verdict measurable, and which cost the corpus nothing because it prints
only when its environment variable is set.

## What a later shift would need

The sieve says the models are there - 22 of them can see a settled
mode - and the wide gate proves the mechanism works, since it won four.
What it lacks is a way to tell a discrete the event iteration has
finished deciding from one it is still deciding. That is a question
about the event loop rather than about the algebraic layer, which is
the shape of a question for a consultation rather than for another
local attempt.

## The floors

Unmoved, and correctly so: 2671 / 0 / 820 / 386 / 722 + 381. The
committed change is an instrument and moves no number.
