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
- **The library's own C, run in a sandbox.** Compiled to WebAssembly
  once, when this compiler is released, and run inside it. Behind a
  feature that is off by default.

Where both could answer, **the one written here wins**. It is the one
this project can promise something about; the sandbox is how to reach
what nobody has written yet, and neither has to wait for the other.
Part of a library written here and the rest delegated is the ordinary
case, not a special one.

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

What is written here needs nothing brought along, and it can be read.
The sandbox costs a dependency and a call that is not free; what it does
not cost is a C compiler on the machine running the model, `unsafe` in
this codebase, or a build of its own on each of three operating systems.

That last part is why the C is run rather than linked to. Linked, a
fault in it takes the whole tool down without a word - which is against
how everything else here fails. Sandboxed, a fault is a trap the host
catches and says out loud, by the name of the function that trapped.

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

The one-dimensional tables are done, and the strings. A table block
keeps its data in a handle - an `ExternalObject` built from the matrix
the model wrote - and asks C for a value at every step. Where the
matrix is in the model, none of that needs C: `flatten/tables.rs`
turns the handle into an expression, a chain of `if`s over the abscissa
with one branch per interval, each the line or the level that interval
carries. What comes out is differentiable, foldable and readable, and
there is nothing left to run. A table read from a file is a different
question - the data is not in the model - and is refused, by name.

The strings are done. `ModelicaStrings_length`, `_substring` and
`_compare` are answered in `flatten/external.rs`, which is the one
place that says which outside names this compiler answers for itself.
A function whose outside name is one of those is not refused: the call
is written as that name and left standing, and the string layer - which
already settles every string at the end of flattening - works it out.
That is how the standard library's transformers pick a ratio out of the
letters of a vector group.

**5. The sandbox, behind a feature.** The library's C compiled to
WebAssembly - by us, at release, so nothing is built on the machine
running a model - and run with `wasmtime`. It fills in whatever step 4
has not, and step 4 may stop wherever it likes.

Off by default: `wasmtime` is a large dependency and a slow one to
build, and a model that never reaches outside Modelica should pay for
neither. Turned on, the calls into it look the same as the ones written
here, because the mechanism in front of both is the same.

`wasmtime` rather than an interpreter because a table is asked for a
value at every step of the run, and an interpreter would be felt there.
The price is build time and megabytes in the binary, which is what the
feature flag is for.

Steps 1 and 2 are independent of each other; everything after 3 may be
done in any order, or not at all.

## What the sandbox is allowed to touch

`ModelicaStandardTables` can read a table out of a file, and models do
that. `ModelicaIO` reads `.mat` files; `ModelicaInternal` reads the
file system and the environment. Under WebAssembly none of that happens
unless a directory is handed over, which means the question - what may
a model read - has to be answered rather than inherited.

The answer here: the directory the model was loaded from and the
libraries in view, and nothing else, unless one is named outright. It is
a decision that costs nothing to make now and is awkward to make later.

## What a model sees while this is unfinished

The same refusal it sees now, and the same one afterwards for anything
nobody has answered - but naming the function, what it takes, and that a
library may be given. A model is never quietly given a number that came
from nowhere.
