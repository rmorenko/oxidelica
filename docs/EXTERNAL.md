# Bodies written outside Modelica

Part of the standard library is not Modelica at all. Tables, string
handling, random numbers, matrix decompositions and file reading are
declared in Modelica and written in C, and the C comes with the library:
2.6 MB of it under `Modelica/Resources/C-Sources`, BSD-3-clause.

A model that reaches one of those is refused here, by name. This is how
that is to change.

Russian version of this file: [EXTERNAL.ru.md](EXTERNAL.ru.md).

## One mechanism, two providers

`external "C" result = ModelicaStrings_length(string)` is a name and a
shape: what to call, what to hand it, what comes back. Nothing about it
says who answers. So the compiler holds one mechanism - a call to
something outside - and the answer may come from either of two places:

- **Written here, in Rust.** Compiled in, present always, the same on
  every machine, covered by tests.
- **Bound to the library's own C.** Found and built where the library
  was fetched, behind a feature that is off by default.

Where both could answer, **the one written here wins**. It is the one
this project can promise something about; the other is a way to reach
what has not been written yet.

## Why not simply bind to the C and be done

Because the binding cannot happen first. A call to `ModelicaStandardTables
_CombiTimeTable_init` hands over an array of doubles, a length, a string
and takes back a pointer that lives from the start of the run to the end
of it. The run here carries one number per slot and answers with one
number. Until it carries arrays, strings and things with a lifetime,
there is nothing to hand over - to C or to Rust either.

That is the same want as the one behind `Modelica.Media.Water`: its
property functions solve an equation by iterating, which needs a body
walked at run time rather than unrolled while flattening. One piece of
work stands behind both.

## Why write some of it here at all

For most of what is written in C, writing it again is less work than
reaching for it:

| What                        | Written here                                     |
| --------------------------- | ------------------------------------------------ |
| `ModelicaRandom`            | xorshift64\* is twenty lines and published       |
| `ModelicaStrings`           | length, substring, compare - and no encodings    |
| `ModelicaStandardTables`    | interpolation the specification sets out in full |
| `ModelicaIO`, MatIO, LAPACK | reaching for it wins: the work is real           |

What is written here needs no compiler on the machine that runs a model,
no `unsafe`, and behaves the same on every platform. What is bound to
brings its own toolchain, its own platform matrix, and a fault in it
takes the whole tool down rather than saying what went wrong - which is
against how everything else here fails.

## The plan, in the order it has to happen

**1. Say what is missing.** The declaration is read as far as `external`
and no further: the name of the outside function and what it is handed
are dropped. Keeping them costs little and makes the refusal name the
thing - `ModelicaStrings_length(string)` rather than "a body written
outside Modelica". Nothing else depends on this, so it comes first.

**2. Values the run can carry.** Arrays, strings, and objects with a
lifetime - a constructor at the start, a destructor at the end. This is
the large piece, and it is chapter 12 of the specification as much as it
is this.

**3. A call to something outside, in the flat model.** Where flattening
now refuses, it puts the call in the model instead: a name, arguments,
and where each answer goes.

**4. What is written here.** Random first (smallest and exactly
specified), then strings, then the tables.

**5. Binding, behind a feature.** Off by default, so the ordinary build
keeps no C toolchain and no `unsafe`. Turned on, it finds
`Modelica/Resources/C-Sources` where the library was fetched, builds it
once, and fills in whatever step 4 has not covered.

Steps 1 and 2 are independent of each other; everything after 3 may be
done in any order, or not at all.

## What a model sees while this is unfinished

The same refusal it sees now, and the same one afterwards for anything
nobody has answered - but naming the function, what it takes, and that a
library may be given. A model is never quietly given a number that came
from nowhere.
