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
- **A local assigned in one branch, 41 models.** The language leaves an
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

## What the measurement asks for

Remembering what a call came to. `(the function, the arguments, the
shapes) -> what it answered` is a pure question while one registry
stands, and the guard for exactly that lifetime already exists - the
same one that remembers what a name means. Eighty-three of every
eighty-four askings would be a table lookup.

Remembering a resolution itself is a wider question: what it comes to
depends on the parameters, the lengths and the loop variables in view
as well, so the key is not the expression alone.

## What is already in place for it

A body the run walks carries arrays, carries records - written as an
array of their members at the boundary, so the walk need not know what
a record is - and answers with as many numbers as it declares. That is
what a body has to be able to do before walking one can ever stand in
for writing it out. It is done and in the tree.
