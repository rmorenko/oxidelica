# What flattening writes out, and what it costs

Almost every function is written out where it is called. That is what
lets a constant fold, a derivative be taken through a call, and a model
come out as one flat list of equations with nothing hidden inside it.

It is also the one thing in this compiler that has ever made it stop
answering. What follows is what was measured, what the measurements
said, and what two of them said wrongly - so that nobody spends another
day finding out the same three things.

Russian version of this file: [EXPANSION.ru.md](EXPANSION.ru.md).

## The models behind it

Two groups of the standard library, 73 models between them, are not
refused for anything they say. They are refused because working them
out does not finish:

- **`Modelica.Media.Water`, 32 models.** Its property functions solve
  an equation by iterating, which is a loop the model decides the
  length of. Such a body is left standing and walked at run time - and
  everything else about it has to reach that point first.
- **A local assigned in one branch, 32 models.** The language leaves an
  unassigned local undefined, and the standard library writes that on
  purpose: a property of water on the boiling curve fills `cp` on one
  side and `cv` on the far side. Taking the value as zero is a
  one-line rule. Applying it opens the chain, and the chain is what
  does not finish.

## Three diagnoses, two of them wrong

**Depth.** The first guess, and wrong. There are depth guards, they are
reached in earnest elsewhere, and raising them changes nothing here:
what runs away is not how deep an expression nests.

**Width.** Right, but not where it was needed. Counting the pieces of
expression one inlining comes to, over the whole library:

| Body                                     | Written out | Largest one |
| ---------------------------------------- | ----------: | ----------: |
| `Frames.TransformationMatrices.resolve2` |         726 |     163 345 |
| `Frames.axesRotationsAngles`             |         106 |     182 592 |
| `Frames.axesRotations`                   |         257 |       5 076 |

A thirty-fold gap between the first two and everything after them. The
mechanism is plain in the source: `resolve2` is `T*v1`, so each element
of the vector is written once per row of the matrix, and a rotation
built of three of those stacks that three deep.

But those models flatten today, and a bound on the size never fires on
the ones that do not: over a hundred seconds of the library, **not one
inlining came to more than two thousand pieces**.

**Repetition.** This is the one. Counting every expression resolution
and the distinct ones among them:

```text
8 500 000 askings, 101 271 of them different (83x over)
  4 825 890  Modelica.Electrical.Spice3.Internal.Bjt.bjtNoBypassCode
  1 264 632  Modelica.Electrical.Spice3.Internal.Bjt.bjtCalcTempDependencies
    217 087  Modelica.Math.exp
```

The same work done eighty-three times over, and the share rises as it
goes: 38x, then 54x, then 62x, then 83x. Not the media library at all -
the transistor models of Spice3.

## What was tried and put back

- **A bound on the size of one inlining.** Never fires. See above.
- **A bound on the size of one assignment inside a body.** The growth
  is spread across many, not sitting in one.
- **Sharing what is written out more than once** - a name and a
  definition instead of a copy. This works and costs nothing: the same
  models flatten, none are lost. It does not help, because a definition
  of its own may only stand where an equation does, and much of the
  work is in parameter values and lengths, which have to come to a
  number where they are written.
- **Walking a body instead of writing it out, past a size.** Written,
  correct, and never reached - there is nothing large enough.

None of these are in the tree. Each was measured, put back, and is
written here so that it is not written twice.

## Remembering what a call came to

This is done. `(the function, how deep the asking is, how many values
are in view, the arguments, the shapes) -> what it answered and what
it checked` is remembered for as long as one class is being
instantiated, which is exactly as long as the parameter values a body
folds with stand still. What the flat models come to does not move by
one equation; the library is read in 41 seconds rather than 60.

The key is longer than the doorway above expected, and each piece of
it was measured rather than reasoned:

- **How deep.** A body that will not come to an end is refused at a
  depth rather than by what it was handed, and the same asking higher
  up may be answered. Without this, five models were lost.
- **How many values are in view.** A body folds with the parameters of
  the class being built, and a caller further along has more of them.
  Without this, two models were lost.
- **Forgetting on a new value.** A parameter settled mid-way changes
  what a body already asked would fold to, so the remembering is
  dropped each time one is.

## What it did not buy

Taking an unassigned local as zero - the one-line rule the 32 models
above are waiting for - still does not finish. With the remembering in
place the library check runs past ten minutes and is killed, where
without the rule it takes 41 seconds. So the repetition was real and
worth removing, and it was not the whole illness: what the rule opens
is not the same work asked again, it is more work.

## Walking the body instead of writing it out

Tried too, and it does not reach this either - for a reason worth
writing down. Retreating to a walk fires on a refusal, and the illness
raises none: the expression simply grows, and `resolve` walks and
clones it for as long as anyone waits. There is nothing for a retreat
to catch.

Retreating on _every_ inlining failure was measured as well: four
models of `ModelicaTest` gain, and four kinds of honest refusal turn
into calls deferred to a run that may not manage them either. That is
a bad trade and it was put back.

What was kept is the narrow half: a body that leaves by a `break` or a
`return` on a condition only the run decides cannot be written out at
all, since which statements run is what the leaving decides. Walking
it is the answer, and the call stands - unless a walk could not carry
what the body answers with, and then the refusal is a refusal after
all. Two models.

## Leaving it undefined rather than giving it a value

The 32 models above wait on `bpro.cp`, a field of a record a function
answers with, which callers genuinely read. Nine more wait on `evd`,
and that one is a different shape worth telling apart: a diode's
`evd` at `Spice3.mo:5730` is a working value of the branch that
computes it, and nothing outside that branch ever reads it. The
outputs `out_current` and `out_cond` are settled by every branch, so
the caller is served whatever `evd` comes to.

That suggests a cure narrower and truer than taking the value as zero:
leave such a variable unbound. The language says an unassigned local
is undefined, and a name carried out of the body is exactly what says
so - a body that reads it later is asking for something that was never
there. Nothing is made up.

Measured, and it is the same wall. It is not the cure that costs, it
is finishing the inlining at all: whichever way the variable is
treated, the body comes out written, and what the body unlocks is what
grows. Two numbers say it:

- The Spice3 four-bit adder **finishes** rather than running away -
  140 seconds, and it comes to rest on a unit mismatch inside the
  library's own transistor code. So the growth here is finite, which
  taking the value as zero never showed.
- The library goes from 45 seconds to past twenty minutes. Killed at
  507 models of 734, seventeen of them in the last nine minutes, and
  MultiBody is in that slow stretch as much as Spice3.

Forty-five seconds is what every measurement in this file rests on, so
that is disqualifying whatever the gain would have been. Put back, and
written here.

## A bound on the growth itself

This is the thing the two above were trying to avoid, and it has now
been tried in both the shapes this file asked for. The rule it is
there to enable was applied first, and it is a rule rather than a
guess: the language starts a function's variables at their `start`
attribute, and a `Real` nobody gave one to starts at zero - so a
branch that says nothing about `bpro.cp` leaves it there.

**A bound on one inlining.** Written, and it does not fire: the growth
is not in any one body, exactly as the width measurement above says.

**A bound on everything written out beneath one call** - which the
width measurement does not cover, and which is what a growth spread
over many small bodies needs. This one works where it is aimed.
`Modelica.Fluid.Examples.Explanatory.MomentumBalanceFittings`, the
first of the 32, stops running away: it finishes in seconds and comes
to rest on a refusal of its own, an arity check on the derivative of
`psat` that nothing had ever reached before.

**A bound on a whole model's writing out.** The one that was left. The
model that runs away in place of the 32 is
`Spice3.Examples.Spice3BenchmarkFourBitBinaryAdder`, and a bound under
one call never touches it: at a budget of 500 it hangs exactly as at
20 000, because the time goes into cloning an expression that is
already large rather than into making a larger one. Only a count over
the whole model catches that, and it can only refuse - there is no one
call to leave standing when what grew is the model.

Measured with the budget swept:

| Budget    | Library   | Models     |
| --------- | --------- | ---------- |
| 8 000 000 | runs away | -          |
| 2 000 000 | runs away | -          |
| 1 000 000 | runs away | -          |
| 500 000   | 3 minutes | 360 of 734 |

So there is a budget that makes the library readable, and it costs 21
models net: 28 of MultiBody go, `Frames.resolve2` being exactly the
body the width table above measures at 163 345 pieces, and the 32 come
through to the next thing that stops them. There is no window between
the two - a budget MultiBody survives is one the adder does not.

What the 32 hit next is worth having found. Each obstacle behind them
was real and is now fixed: a chain of type aliases read where each
name was written, a local's declaration value reading a constant of a
class, a call subscripted where it stands, a call the run walks
subscripted the same way. Those are in the tree and stand on their own.
The pile now stops at an expression that nests deeper than the
compiler follows, which is the same illness one layer down.

Neither the rule nor the bound is in the tree. Both are one edit each
and are written here rather than kept, since a compiler that cannot
read its own library is worse than one that refuses 32 models of it,
and a bound that buys those 32 by losing 28 others is not a bargain
either.

## What is already in place for it

A body the run walks carries arrays, carries records - written as an
array of their members at the boundary, so the walk need not know what
a record is - and answers with as many numbers as it declares. That is
what a body has to be able to do before walking one can ever stand in
for writing it out. It is done and in the tree.

## A function handed as an argument: defunctionalization

Eighteen models of the register stop at one language feature:

```modelica
x_zero = Modelica.Math.Nonlinear.solveOneNonlinearEquation(
           function f_nonlinear(A=A, w=w, s=-y_zero), x_min, x_max);
```

The parser already reads this - `Expr::Call(PARTIAL_CALL, [Ref(name),
NamedArg..])`, `ast.rs` - and `arrays.rs` refuses it for want of
anywhere to put a function. What is missing is not a kind of value.

A function value has exactly one sink in this language: an argument of
a function. So every pair of "who was handed over" and "to whom" is
visible where the call is written, and the receiving function can be
specialized instead: a copy of it with the function input gone and
ordinary numeric inputs in its place, and every call to that input
rewritten into a direct call of the target. Nothing new survives past
that boundary, and neither the walk nor the code needs to hear of it.

The receiving function of these eighteen never inlines - its body is
Brent's method, a `while` on simulated values - so it stands as a call
and is walked at run time. That is the shape the walk was built for,
and it takes numbers only, which is the reason a `Value::Function`
would have to be erased at the same boundary anyway.

Two potholes lie on the way, both wider than these eighteen models:

- A body that shouts through `Streams.error` is gathered as something
  to carry, and the gatherer refuses it for taking a String and
  returning nothing - which kills the flatten rather than the branch.
  An external body with no outputs is already nothing when inlining;
  the same rule belongs where bodies are carried.
- A local declared `constant Real eps = Modelica.Constants.eps` is a
  name the walk's frame has never heard. Class constants want
  substituting into a carried body's bindings, the way lengths already
  are.

And one honest ceiling: a standing call is opaque to differentiation,
so a model wanting the answer under `der()` stays out. The models here
ask for it algebraically.
