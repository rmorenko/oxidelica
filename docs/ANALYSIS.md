# The architecture, what stands in its way, and the way out

Written 2026-08-24, and kept up to date as the stages land. Measured
against MSL 4.1.0: 767 example models, of which **419 flatten and 89
run**, and — counting only the ones written to be run — **640 runnable,
of which 330 flatten and 86 run** (`oxidelica library check`). The
register below was taken at 400/80, before stage one; the counts in it
are what each class cost then. What follows is an honest register of what
stops the rest, what two other compilers do about the same problems,
and the order in which to take them on.

Russian version of this file: [ANALYSIS.ru.md](ANALYSIS.ru.md).

## How this compiler is put together

Four crates, about 52 500 lines, no dependencies outside the standard
library:

```text
oxidelica-parser (~20 000 lines)
  lexer.rs                — tokens
  parser/                 — recursive descent: classes, declarations,
                            equations, expressions, statements
  ast.rs                  — one tree for everything: Expr (24 variants),
                            Component, EquationItem, ClassDef
  flatten/                — ONE pass that does all of it:
    instantiate.rs (3648) — instantiation, extends, modifiers,
                            redeclare, inner/outer, conditional parts
    clocks.rs      (2514) — clocked partitions and state machines
    algorithms.rs  (2492) — algorithm sections executed symbolically,
                            functions inlined
    arrays.rs      (1865) — arrays expanded into scalars
    names.rs       (1618) — name resolution, const_eval
    connections.rs        — connect: potentials, flows, streams
    mod.rs         (1397) — orchestration, settle_sizes, MAX_DEPTH=32
  check.rs         (1550) — verification of the flat model

oxidelica-sim (~6 500 lines)
  compile.rs       (2355) — matching equations to unknowns by augmenting
                            paths, Pantelides with dummy derivatives,
                            tearing, a plan laid out in stages
  code.rs           (740) — bytecode for expressions
  walk.rs           (534) — an interpreter for function bodies that
                            would not inline
  events.rs, solvers/ (dopri45, bdf, rk4), linear.rs, symbolic.rs

oxidelica-cli, oxidelica-ide — the command line and the editor
```

The decisions that shaped it, and what each one costs:

| Decision                                                     | What it buys                                     | What it costs                                                                                                             |
| ------------------------------------------------------------ | ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------- |
| Everything is a scalar: arrays are expanded while flattening | a simple back end, no tensor IR                  | size blows up (Spice3: 8.5M expression resolutions, 83x repeated — see EXPANSION.md); `MAX_DEPTH=32` stands in as a guard |
| Functions are written out where they are called              | differentiation through a call, constant folding | Media and Spice3 will not inline (loops the model decides); `walk.rs` answers those without folding or differentiating    |
| One `flatten` pass, no intermediate IR                       | little code                                      | nowhere types and dimensions are known before expansion; shape errors surface deep and late                               |
| `Expr::Ref(String)` — names as strings, dots inside          | simple                                           | no identities or scopes; longest-prefix searches (`flat_name`), brittle inner/outer special cases                         |
| No typing pass of its own                                    | less code                                        | `type mismatch in sample(...)`: overloads settled ad hoc, clocked Integer and Boolean signals refuse                      |
| A component's condition must be constant at compile time     | simple                                           | 34 models: a `useHeatPort`-shaped flag arriving as a parameter stops flattening                                           |

What is already good and needs no rework: the lexer and parser (99.4% of
MSL files read), connect semantics (potential, flow, stream, expandable),
inheritance and redeclare, state machines, the basics of synchronous
clocks, Pantelides with dummy derivatives and tearing in the simulator,
and events.

## The register

Taken at 400 flatten and 80 run, before stage one: 767 examples = 400
flatten (80 run + 320 that will not) + 367 that will not flatten.

### What stops flattening (367 models)

| Class                             | Models | What it is                                                                                                                     | Where it comes from                                                                                                    |
| --------------------------------- | -----: | ------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------- |
| **F1. Dimensions not constant**   |    ~50 | `dimension of X is not a compile-time constant`, `flexible size :` — a size that depends on `Medium.nC` or on a redeclare      | no typing pass: sizes are worked out during expansion, before a redeclare has been substituted                         |
| **F2. Conditional components**    |     34 | `condition of component ... is not a compile-time constant` — the condition reads a structural parameter of an enclosing class | `instantiate.rs:592` demands a constant then and there; it should be evaluated on the instance, after modifiers        |
| **F3. Subscripts**                |     34 | `subscript 1 is outside an array of 0`, indices that will not settle                                                           | a consequence of F1: the array got a size of nought because the size never settled                                     |
| **F4. Expression depth**          |     32 | `nested deeper than the compiler follows` (MAX_DEPTH=32)                                                                       | inlining Media builds expressions deeper than 32; raising the bound costs 25% (64) or the process (96)                 |
| **F5. Clocks: the gaps**          |     30 | `previous` with no clock across a redeclare boundary; clocked Integer and Boolean; one-argument `sample(u)`                    | clocks travel over flat names, so a subcomponent whose class was replaced never hears about the clock                  |
| **F6. Functions with no body**    |     25 | Spice3 `mosCalcNoBypassCode`, `PartialTwoPhaseMedium.setState_*`                                                               | the inliner gives up and the walk cannot carry records that large; dispatch of a replaceable function is unresolved    |
| **F7. A branch with no value**    |     15 | `assigned in one branch only` — outside a function the language does demand a value first                                      | for 8 of them (Spice3) it is a false refusal: an array in a neighbouring branch takes the start away from every scalar |
| **F8. inner/outer gaps**          |     11 | `outer World world` in a class instantiated before the `inner` exists                                                          | order of instantiation: `inner` is only looked for up a tree already built                                             |
| **F9. Array shapes**              |   11+8 | `array of shape [2] where a scalar is expected` (connect onto `u[1]`, `u[2]`), `given a run of 2 and 1 value(s)`               | connect expanded onto one element of a block's input array                                                             |
| **F10. Tables**                   |     ~8 | splines and periodic extrapolation are not written                                                                             | tables.rs interpolates linearly                                                                                        |
| **F11. External functions**       |     ~6 | `ModelicaStandardTables_*`, `FileSystem.stat` — `external "C"`                                                                 | external.rs answers only `"builtin"`                                                                                   |
| **F12. Parser, the small change** |    3-5 | `{expr for j in 1:nv, i in 1:nu}`, a connect subscript in one particular setting, `inverse(...)` with `=`                      | expressions.rs:220, equations.rs                                                                                       |

### What stops a run (320 models that flatten)

| Class                            | Models | What it is                                                                                                                |
| -------------------------------- | -----: | ------------------------------------------------------------------------------------------------------------------------- |
| **R1. Unbalanced**               |    211 | mostly helper models — parts of examples with open pins and nothing to drive them                                         |
| **R2. Parameters with no value** |   26+8 | the same thing said differently: a helper is parameterised by whatever holds it                                           |
| **R3. Structurally singular**    |     21 | quasi-static circuits and transformers: an equation constrains no state; wants alias elimination and better matching      |
| **R4. Clocked balances**         |     ~7 | BackSample: the input of one partition gets no equation from the partition feeding it                                     |
| **R5. Two equations for der(x)** |      7 | `fixed = true` on a start beside an equation; wants a stated order of precedence at initialization                        |
| **R6. Initialization**           |    ~10 | `fixed at X but the constraints say Y`, an initialization that is not square; wants an initial-value solver with homotopy |
| **R7. Singular Jacobians**       |     ~8 | FluidHeatFlow loops: wants better tearing and a Newton with a line search                                                 |

### The metric, corrected (stage 0, done)

`library check` counted as an example anything with `.Examples.` or
`.Test` in its name — helper classes included, which nobody runs on
their own. It now prints a second line: **runnable examples**, those with
an `experiment` annotation or the `Icons.Example` icon, inherited through
a template or not. Measured on MSL 4.1.0:

```text
example models: 767, of which 419 flatten and 89 run
runnable examples (experiment or Example icon): 640, of which 330 flatten and 86 run
```

What that says. Of the 127 that are not runnable, 87 flatten and 3 run:
those 84 refusals were never targets, and they no longer hide the work.
But 236 runnable models flatten and will not run, so the first guess —
that R1 and R2 are all helpers — was too kind: helpers account for about
84 of the 320. The floor in `library_floor.sh` now holds both pairs.

## What the other two do

### OpenModelica (NFFrontEnd and NBackEnd)

A mature compiler, some 39 000 commits, written in MetaModelica. Its new
front end, by file:

```text
Lookup/InstNode/ClassTree — names resolved to nodes, an instance tree
NFInst                    — instantiation over nodes, not strings
NFTyping/NFTypeCheck      — a full typing pass: types, dimensions and
                            variability, before anything is expanded
NFCeval/NFEvalFunction    — constant evaluation and a function interpreter
NFFlatten                 — flattening of a typed tree
NFScalarize               — scalarization, separate and optional
NFConnectEquations        — connect after typing
NFSimplify/NFInline       — simplification and inlining as passes of their own
```

What is worth taking:

- **Typing before expansion.** Dimensions are worked out on a typed tree
  where redeclares have already been substituted. That is F1 and F3 gone.
- **An instance tree of nodes rather than strings** (NFInstNode):
  inner/outer, lookup and protected all become local operations.
- **Scalarization is optional**, so large arrays need not blow up memory.
- **The function interpreter belongs to the front end** (NFEvalFunction)
  rather than being a way out: evaluating a function at compile time is
  a call into the interpreter.

### rumoca (CogniPilot)

Rust, under active development, aiming at Modelica as a semantic front
end for symbolic ecosystems. Every phase is a crate:

```text
rumoca-phase-parse        → AST
rumoca-phase-resolve      → identities, scopes, name resolution
rumoca-phase-typecheck    → types, dimensions, structural parameters
rumoca-phase-instantiate  → extends and modifiers
rumoca-phase-flatten      → hierarchy, connect, residual equations
rumoca-phase-dae          → variable classification, the DAE
rumoca-phase-structural   → BLT, incidence, matching, an IC plan
rumoca-sim-core           → initial values solved, then integration
rumoca-phase-codegen      → templates (CasADi, JAX, and so on)
```

What is worth taking:

- **A resolve pass with identities** before anything else.
- **IR boundaries between phases**, so each is testable on its own, with
  an MSL parity gate in CI.
- **The initialization plan as an artefact of its own** — which is what
  R5 and R6 are asking for.
- **Contract tests against the specification**.

### Where this compiler is already ahead

No dependencies and a single binary; state machines and synchronous
clocks, which rumoca has little of; stream connectors and expandable
connectors in full; a diagram editor; and prose documentation with 95%
line coverage held in CI.

## The way out

Ordered so that every stage pays for itself and the later ones get
cheaper for the earlier ones having happened. Model counts come from the
register above.

### Stage 0. An honest metric (a day) — done

`library check` counts runnable examples apart, and the floor holds
both pairs.

### Stage 1. The small change in flattening (days) — done

1. **F12**: a comprehension over several iterators — +2.
2. **F9**: a `when` giving a whole array (`y = u` between vectors) — +1.
3. **Part of F5**: the one-argument `sample(u)` — **+10 flatten, +9 run**.
   It turned out not to be dispatch but two operators sharing a word: the
   event `sample(start, interval)`, which is Boolean, and 16.3's
   `sample(u)`, which is what it read and takes its clock from the
   equation it lands in.
4. **The rest of F7**: the one-line change works, but the library check
   went from three and a half minutes to twenty-one for no model gained -
   the eight Spice3 models then meet a function nothing can inline.
   **Put back until stage 4.**

Result: 400/80 → **413/89**; runnable 313/77 → **326/86**.
What surfaced: 11 Digital models subscript a table with a discrete
variable (`NotTable[x]`), which is F1/F3 and waits for stage 3.

### Stage 2. One pass, and what it cannot see (measured, then reopened)

This was written as a resolve pass with identities, and costed at one to
two weeks on the reasoning that stage 3 would need it. Measured before it
was started, that reasoning did not hold: of 354 refusals at flattening
**none** are about resolving a name, and of 324 at running **13** are,
all one narrow shape. `lookup` already resolves names correctly and
remembers what it found. Identities would have bought about thirteen
models.

What the same measurement did turn up is a different thing sharing the
same neighbourhood, and it is worth more. The compiler builds a model in
one pass, in the order things are declared, so a question asked partway
through cannot be answered from what has not been reached yet:

- **A condition reading an `inner` declared further down.** Every
  animated part of the multi-body library is written `if
world.enableAnimation and animation`, and a diagram declares its
  components in the order they were drawn. Roughly fifty models.
- **A record's inherited fields.** `record Mos1Calc extends Mos.MosCalc;
end` declares nothing of its own; reading it for its own components
  alone says it has no fields. Nine Spice3 models, and the same shape
  under the media library's thermodynamic states.
- **A length read off a value declared above.** `Impedance
impedance(cellData = cellData)` hands a record over whole, and a `:`
  among its fields takes its length from the field it was handed - which
  belongs to a class the pass has not come back to.

Each was tried on its own and each is correct in the small: the tests
written for them pass, and fail without them. Each also fails to pay,
and for the same reason. The record fix wins nine models past their
refusal and loses `ShowImpedance` to the length it uncovers, twice
measured at −1/+0. The condition fix lets fifty multi-body models past
the condition and into the inlining of their visualisers, which took the
library check from three and a half minutes to over an hour.

So the stage is not a resolve pass. It is **an order of instantiation
that does not depend on the order of declaration**: measure what a class
holds - shapes, constants, the `inner` instances - before its components
are built, and let the questions asked during the build be answered from
that. The three shapes above are one stage's work together and none of
them alone.

Expect +60 to +80 flatten, and the multi-body models to stop being
refused for something that is not their fault.

### Stage 3. Typing and dimensions before expansion (started)

A pass over the typed instance tree where every component knows its type,
its dimensions as numbers and its variability, before scalarization; and
where a redeclared `Medium` is already in place. This closes **F1 (~50),
F3 (34) and most of F2 (34)**.

Six layers of it were taken one at a time, each measured on the library
before and after, each with a test that fails without it:

1. A package holds its base's constants to what the `extends` said -
   `extends PartialMedium(nC = 2)`, which is how every medium is written.
2. A package handed on by its own name is the one it was replaced with -
   `Port one(redeclare package Medium = Medium)`.
3. A constant may be the length of another: `nC =
size(extraPropertiesNames, 1)`.
4. An equation between two empty arrays says nothing rather than being
   refused for `[0]` against `[]`.
5. A replaceable package a base declared is in view of what extends it -
   `Medium.AbsolutePressure` in a class extending `PartialSource`.
6. A string a body writes in one branch starts empty, as 3.7 says.

Result so far: 415/89 → **419/89**, runnable 328/86 → **330/86**. The
first four layers won four models; the last two won none directly and
moved fifty-six models one blocker further along, which is what the
layers are for.

What is left of this stage runs into stage 2's finding: the media
library's records are built by `redeclare record extends`, and reaching
their fields is the same instantiation-order problem. The remaining
+80 to +110 is real but sits behind that.

### Stage 4. The function interpreter as a first-class citizen (a week)

`walk.rs` grows into something NFEvalFunction-shaped — records in and
out, arrays whose length the model decides, `while` — and const_eval
calls it. Inlining becomes an optimization rather than a necessity, and
memoizing what inlining resolves (83x repeated in Spice3) takes the
pressure off MAX_DEPTH. Closes **F4 (32), F6 (25)** and part of F10/F11.

Expect +50 to +60 flatten.

### Stage 5. Initialization as a plan of its own (a week)

Initial equations, fixed starts and homotopy in one system, with the
order of precedence the specification states. Closes **R5 (7) and
R6 (~10)**, and helps R3 and R7.

### Stage 6. Clocks over the instance tree (a week)

Clocks travel the instance tree rather than flat names, so a redeclared
subcomponent hears about them (F5); and a partition reading another's
value takes it from the tick before, which is what `backSample` means (R4).

Expect +15 to +25 across both numbers.

### Stage 7. The numeric back end (as needed)

Alias elimination before matching (the `v = p.v - n.v` chains) shrinks
the systems and answers part of R3; a line search or Levenberg-Marquardt
in Newton answers R7; the standard tables written natively, splines and
periodic extrapolation included, answer F10 and F11.

### What to expect

| Stage                  | Cost      |                               Flatten |    Run |
| ---------------------- | --------- | ------------------------------------: | -----: |
| 0. Metric              | a day     |                                     0 |      0 |
| 1. Small change        | days      |                               **+13** | **+9** |
| 2. Instantiation order | 1-2 weeks |                                +60-80 |     +5 |
| 3. Typing              | 1-2 weeks | **+6** so far, +80-110 behind stage 2 |    +20 |
| 4. Functions           | a week    |                                +50-60 |    +15 |
| 5. Initialization      | a week    |                                     0 | +15-20 |
| 6. Clocks              | a week    |                                   +10 |    +10 |
| 7. Numeric             | as needed |                                    +5 | +20-40 |

Stages 0, 1 and part of 3 are done: 397/79 at the start of this work,
**419/89** now, and 313/77 → **330/86** of the ones written to be run.
The estimates for stage 2 and 3 are the ones that moved, and they moved
because they were measured rather than reasoned about.

After stages 0 through 6: **around 570-600 flatten and 180-250 run** of
767, which is over half of the runnable ones. What is left after that is
a long tail of Media and Fluid particulars, worked through one at a time
on an architecture that can carry them.

## How to move

1. **No large rewrite.** Every stage is a series of commits, each passing
   preflight and each leaving the numbers no worse.
2. **Strings stay in the messages**; identities are internal.
3. **Every stage closes with a test that fails without it**, and a
   library check whose numbers go in the commit message.
4. **Scalarization stays** through stage 3: the back end works on scalars
   and that is fine at the size of these examples. A tensor back end is a
   later conversation.
5. **ModelicaTest is not the target.** Modelica first; the test suite is
   a bonus.

## The register, taken again at 639 flatten and 323 run

Measured by `scripts/refusals.sh <library> both` on the tree that reads
a table from the seat it was written into. 1043 example models: 639
flatten, of which 323 run. The runnable subset (912 with an
`experiment` annotation or an `Example` icon) stands at 549 and 320,
and it moved by exactly what the whole library moved - the seat fix
freed no model that was outside it.

### What stops flattening, the top of it

| Count | What it is                                                                         |
| ----: | ---------------------------------------------------------------------------------- |
|    25 | a dimension is not a compile-time constant                                         |
|    24 | `Range(0, None, size(...) - 1)` where a scalar is wanted                           |
|    24 | an Integer stands where a Boolean is needed                                        |
|    22 | a function is missing an argument                                                  |
|    20 | `Range(N, None, N)` where a scalar is wanted                                       |
|    18 | a function handed as an argument with some of its inputs filled in (`f_nonlinear`) |
|    16 | a flexible size with nowhere to read its length from                               |
|    16 | the condition of a component is not a compile-time constant                        |
|    14 | an external C function this compiler does not answer for                           |
|    11 | a table this compiler answers for, whose data it still cannot see                  |

The two kinds that were a heap a few commits ago are gone from this
list entirely: `if` with no `else` and `for` in a branch nobody can
settle, both 0. The three flexible-size and range kinds above are one
family read three ways by the message, not three problems.

### The external world, by name

Seventeen models stop at a function written outside Modelica, and they
are not one wish but two. What a compiler could answer for itself:

| Times | Name                                                          |
| ----: | ------------------------------------------------------------- |
|     4 | `ModelicaInternal_stat` - does a path exist, and is it a file |
|     2 | `ModelicaInternal_readLine`                                   |
|     1 | `ModelicaInternal_readFile`                                   |
|     1 | `ModelicaInternal_getcwd`                                     |
|     1 | `ModelicaStrings_scanInteger`                                 |
|     1 | `ModelicaIO_readMatrixSizes`                                  |
|     1 | `ModelicaIO_writeRealMatrix`                                  |

Nine models, all of them file system and string work this compiler
already does elsewhere in Rust: the answer is to write them here, not
to link anything. What needs a real library, or a model that hands a
compiled function over:

| Times | Name                                                              |
| ----: | ----------------------------------------------------------------- |
|     3 | `dgesvd`, `dgelsy`, `dgees` - LAPACK, in FORTRAN                  |
|     3 | `mydummyfunc` - a table the test suite supplies from C of its own |

Six models, and three of those are ModelicaTest handing itself a table
through a C pointer, which is not a target. So the road to running
without any C at all is nine models wide, and the LAPACK three are the
only ones that would need a numerical library rather than a morning.

### The eleven tables that still refuse

All of them two-dimensional (`CombiTable2Ds`, `CombiTable2Dv`) or a
battery cell whose data is a `CombiTable1D` written into the model.
The file seat is now read correctly; what stops these is further in.

### The three flexible-size readings are three roots, not one

The register groups them by how the message reads, and the last one
claimed they were one family read three ways. Traced, one model to a
reading, they are not:

| Reading                                              | Model traced                       | What is actually underneath                                                                                                                                   |
| ---------------------------------------------------- | ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Range(1, None, n - 1)` where a scalar is wanted, 24 | `AST_BatchPlant.Test.OneTank`      | `Wb_flows[1:n-1]` with `n` known and settled - the slice resolves, and what stops the model is the next thing along, an array of one where a scalar is wanted |
| flexible size with nowhere to read a length, 13      | `Tables.CombiTable1Ds.Test20`      | `columns[:] = 2:size(table, 2)` on a table read from a **MAT v5** file; the reader here knows level 4 and text, and v5 begins `MATLAB 5.0 MAT-f`              |
| dimension is not a compile-time constant, 25         | `Noise.DrydenContinuousTurbulence` | `Hw.x_start`, a state vector sized by a filter order worked out elsewhere                                                                                     |

Three barriers, three places. The middle one is the cheapest and the
most self-contained: a MAT v5 reader would answer the whole
`CombiTable*.Test2x` family, which is where the flexible-size count
comes from. The first is not really a flexible-size problem at all -
the slice settles, and the message names the wrong thing.

A slice by a settled range can be resolved on the scalar path as well
as the array path, and that was tried: it moves the barrier one step
along in those 24 models and frees none of them, so it is not in the
tree. What frees them is whatever stands behind it.

### The wall behind the settled slice, traced through

Resolving a settled slice on the scalar path frees no models, and now
it is known why. Traced through - the change applied locally, one
model run to its next death - the barrier one step along is:

```text
an array of shape [1] is used where a scalar is expected, beginning
Bin(Div, Bin(Div, Bin(Add, roughnesses[1], roughnesses[2]), 2.0),
         Ref("pipe.flowModel.diameters"))
```

That expression is not in the model. It is the body of
`Detailed.pressureLoss_m_flow`, `Fluid/Pipes.mo:2530`:

```modelica
Real Delta(min=0) = roughness/diameter "Relative roughness";
```

with `roughness` bound to `(roughnesses[1:n-1] + roughnesses[2:n])/2`
and `diameter` to `diameters`, both whole. So the two walls are one
place: the slice resolves, the local `Delta` divides an array of one
by a name, and `Delta` is declared `Real`.

Two things are known about that point, both measured rather than
supposed. The shapes are there - `pipe.flowModel.roughnesses` is
`[2]` and `.diameters` is `[1]` in the table at the moment of the
refusal - and `n` is settled under both spellings. So this is not a
model whose forms were never measured. What happens instead is that
the whole argument expression goes down the scalar path, where a
range is refused for being an array; resolving it there produces an
`Array[1]` that then meets a scalar in the same division.

The way through is to make `diameters` an array of one at the same
moment, so the division is array-to-array and `Delta` is the array of
one it should be - which is vectorization of a scalar function over
several arguments at once, inside an inlined body. That is a bigger
change than a slice, and it is the change these 25 models want.

### The slice wall, measured to the fifth layer

The advice was to deliver the shapes and change nothing about how a
range is handled. Measured, that is not where it stands either.

Three things were established, each by a probe on the failing model
rather than by reading:

- At the point of death the shape table is **empty** - zero keys -
  because the death happens inside a function body, where a function
  has no shapes of its own. Not "the drain had not reached
  `diameters`": there is no table to reach.
- The scope is `WallFriction.Detailed.massFlowRate_dp_staticHead`,
  not the `pressureLoss_m_flow` the trace suggested. Both hold the
  same line, `Delta = roughness/diameter`.
- The call that does go through `expand_call` -
  `pressureLoss_m_flow` - arrives with its arguments already
  `Array[1]` where they should be, so the vectorizing hand-out is
  reached and works. The one that dies never comes past that gate at
  all.

So the shape delivery was tried too: offering every parameter of a
class to the drain rather than only its constants, so a binding
written on a name declared below it can see it. The numbers do not
move. What dies is a body already inlined, in a scope that has no
shape table by construction, and the array in it was built by honest
hands out of one settled slice and one bare name.

Five layers in, the count is still 665 and 333. The thing worth
fixing first is not any of the walls but the birth of the lie: an
input declared scalar bound to an array. That refusal should carry
the name of the input and the function - `input 'diameter' of
'massFlowRate_dp_staticHead' is scalar and was handed an array of 1`

- rather than an anonymous shape four layers down. It frees nothing
  by itself, and it would have made this whole excavation one run
  long.

That guard was written, run and taken out again, which is worth
knowing before it is written a second time: "declared with no
dimensions" is not the same as "takes one number". A record input has
a shape made of its fields, and an input whose type is an array type

- `Real3 a` where `type Real3 = Real[3]` - is declared bare and takes
  nine numbers. Both are legitimate, both trip the guard, and both are
  in the test suite, which is the good outcome. A real guard has to ask
  the type what it is rather than reading the declaration.

### The declaration-order theory, tried by experiment

The account of the slice wall that fitted everything else said this:
the shapes are drained in declaration order, so a binding written on
a name declared below it sees that name without a shape and takes it
for a scalar. It named a prediction to be caught by, and the
prediction is cheap: move the declaration and the models free
themselves with the compiler untouched.

Tried, on a copy of the library, three placements of the same
declaration:

| Where `diameters` stands                    | What the model says             |
| ------------------------------------------- | ------------------------------- |
| As shipped, above the binding that uses it  | the slice refusal               |
| Moved below `dp_fric_nominal`               | the same refusal, word for word |
| Moved to the top of the `protected` section | the same again                  |

And the shape table at the moment of death holds **zero** keys, not
"some but not this one": the death is inside a function body, and a
function has no shapes of its own by construction. Both halves of
the theory are answered by measurement rather than by argument.

So declaration order is not the mechanism, and neither is a late
drain. What is left is what the earlier excavation already said: an
input declared as one number, bound to an array, inside a body that
was inlined whole. The lie is born at the binding, and the shape
table it would have to be caught by does not exist where it dies.

### The scalar-input guard, measured

Written a second time, asking the type rather than the declaration -
a record input has a shape made of its fields, an input whose type is
an array type is declared bare and takes several - and both of those
are answered correctly. On the failing pipe model it says exactly
what a week of digging said:

```text
input `dp` of `WallFriction.Detailed.massFlowRate_dp_staticHead`
takes one number, and it was handed an array
```

And then the library check: **633 flatten, down from 666**. Thirty
three models refused for something they had been doing all along.
The guard is right about the pipes and wrong about thirty three
other things, and no synthetic model reproduces any of them - the
vectorizing hand-out catches every shape a test can write, which is
why the guard looked safe on a dozen tries.

So it is not in the tree either. What it did earn is the diagnosis
it printed on its way out, which is written above rather than in the
code: the wall is `dp`, an input declared as one number, bound to a
whole array by a caller. Anyone taking this up again should start by
finding which thirty three models the guard is wrong about, since
that is the same question as what the vectorizing hand-out means by
an array in a scalar seat.

## The register after the redeclared functions

Taken by `library check --refused` on the tree at 666 flatten and
333 run. The shape of the list has changed twice over since the last
one.

| Count | What it is                                                                 |
| ----: | -------------------------------------------------------------------------- |
|    40 | a body nothing could inline, walked at run time, that the walk cannot take |
|    25 | `Range(1, None, n - 1)` where a scalar is wanted - the slice wall          |
|    16 | the condition of a component is not a compile-time constant                |
|    13 | an argument that must be dimensionless carries a unit                      |
|    12 | `previous` with no clock across a redeclare boundary                       |
|    12 | a name with no declaration above it                                        |
|    11 | a subscript outside its array                                              |
|    11 | a `connect` between different numbers of connectors                        |

Two kinds that stood near the top are gone or nearly so.

**A function missing an argument: 22 to 6.** Every pump in the
library asks for its curve as `redeclare function flowCharacteristic
= quadraticFlow(V_flow_nominal = ..., head_nominal = ...)`, and the
modifiers were being read and dropped. Of the six that remain, four
are `PartialMedium.dynamicViscosity` and two its table-based
cousin - a different question, a medium function called where the
medium never filled its inputs in.

**A dimension that is not a compile-time constant: 25 to 0.**
Nothing was aimed at it; it fell to `size(a, 1) - 1` being read as a
length, which was aimed at thirteen models of a different kind.

The new top is the runtime wall, 40 and rising - which is what
happens when models that used to stop at flattening now get far
enough to be walked. The slice wall at 25 is unchanged and now
carries a written diagnosis rather than a guess.

Of the eighteen models the specialization freed, four are back under
`SolveOneNonlinearEquation`, each on a different further thing: a
`size` of a name with no shape, a unit on a logarithm. They are not
one family any more.

## What the forty models met next

Forty models freed at once moved the flatten count by forty and the
run count by nothing, which asks a question of its own: did they all
stop at the same next thing, or scatter?

Scatter, mostly - but with one new family at the top of the running
half of the register:

| Count | What it is                                                                            |
| ----: | ------------------------------------------------------------------------------------- |
|    18 | an unknown variable                                                                   |
|    13 | `cannot evaluate parameters [... = Medium.X_default, ... = waterBaseProp_pT(...)[5]]` |
|    11 | a singular Jacobian in an algebraic loop                                              |
|     9 | an unknown function                                                                   |
|     9 | an unbalanced model                                                                   |
|     9 | initialization that is not square                                                     |

The thirteen are new and they are one thing: a parameter whose value
is a field of a record a medium function answers with -
`waterBaseProp_pT(p, T, 0)[5]` is the fifth field of an
`IF97BaseTwoPhase`. The bodies are carried to the run now, which is
the win; the parameters are wanted before the run starts, which is
the next barrier. That is a real family and a plausible next take.

The rest is a long tail of numerics - singular Jacobians, unbalanced
models, loops that do not converge - which is what a compiler that
flattens two thirds of a library and runs a third of it looks like
from inside.

### The tails

**A function missing an argument: 6 left**, from 22. Four are
`PartialMedium.dynamicViscosity` and two its table-based cousin: a
medium function called where the medium never filled its inputs in,
which is not the pumps' problem and not the same fix.

**The eighteen the specialization freed** are no longer a family.
Four are back under `SolveOneNonlinearEquation`, each stopped on
something else entirely - a `size` of a name with no shape, a unit
on a logarithm - and the rest have gone on into the running half.

## The register after the named arguments

Taken at 717 flatten and 333 run.

### What stops flattening

| Count | What it is                                                  |
| ----: | ----------------------------------------------------------- |
|    16 | the condition of a component is not a compile-time constant |
|    15 | an argument that must be dimensionless carries a unit       |
|    13 | an equation between shapes, `pipe.statesFM[n].phase = ()`   |
|    12 | a loop whose trip count is not settled                      |
|    12 | `previous` with no clock across a redeclare boundary        |
|    12 | a name with no declaration above it                         |
|    11 | a subscript outside its array                               |
|    11 | a `connect` between different numbers of connectors         |

The slice family is gone from this list entirely - it was 25 and 10
in two readings a few commits ago, and the named-argument arm took
both. What is left of that neighbourhood is the thirteen at the
third row, which is the same pipes one step further along: a state
record whose `phase` field has nothing on the right of it.

### What stops running

| Count | What it is                                                 |
| ----: | ---------------------------------------------------------- |
|    18 | an unknown variable                                        |
|    11 | a singular Jacobian in an algebraic loop                   |
|     9 | an unknown function                                        |
|     9 | an unbalanced model                                        |
|     9 | initialization that is not square                          |
|     9 | `cannot evaluate parameters [... = Medium.X_default, ...]` |
|     8 | an algebraic loop that diverged                            |

Run has stood at 333 for four series while flatten went up by 51,
and this is where the water is: no single dam, but two of these are
of a kind - the eighteen unknown variables and the nine unevaluated
parameters both say a name reached the run that the run has no slot
for. The rest is numerics, which is a different trade.

### The `useDamperCage` chain, walked to its end

Sixteen models stop at "the condition of a component is not a
compile-time constant". Walked with the probe rather than fixed one
link at a time, the chain has four links and a floor:

1. **The condition of a component in a connector.** `parameter
Boolean useDamperCage(start = true)` and a `HeatPort ... if
useDamperCage` beside it. A `start` is where a parameter stands
   when nobody says otherwise, and a condition has to be settled
   before anything can be handed down to it.
2. **An `if` equation with no `else`,** on `not useDamperCage`, in
   the machine itself. Same name, different seat: this one refuses
   because the two branches would give different numbers of
   equations.
3. **The machine's own parameter,** `useDamperCage(start = true)`,
   declared forty lines below the `extends` that already uses it.
4. **The floor: a field of an inherited record.** The model writes
   `smpmData(useDamperCage = false)`, and `SM_PermanentMagnetData`
   does not declare that field - it extends `SM_ReluctanceRotorData`,
   which does. The modifier lands on a field the record only has
   through its base, and never settles at all.

So the sixteen do not want four fixes: they want the fourth, and the
first three fall out of it. That is the shape the chain rule was
written for - and the reason the first link, taken alone, measured
zero and was reverted.

A model of this shape freed by hand is one screen long: a record
extending another, a field modified from outside, a condition on it.

The fourth link is taken (a modifier at the site now outranks one an
`extends` handed down, with a test on its own number). The chain did
not open, and the probe says where it stands now: `smpmData.
useDamperCage` never reaches the settling round at all - not
"settles to nothing", but is never offered. Every other field of that
record is; this one is declared two levels up, in
`SM_ReluctanceRotorData`, which `SM_PermanentMagnetData` extends
through `InductionMachineData`.

Three synthetic models of that shape - one level of inheritance, two,
three, with the field modified from the site - all settle correctly.
So the shape alone is not it, and the next probe belongs where a
record's fields are turned into components rather than where they are
read.

## The connect wall the machine chain opened onto

The `useDamperCage` chain freed its models into a new top of the
flattening register: `connect between 2 and 1 connector(s)`, 21
models, up from 11 before the chain.

The probe reads it in one line. `connect(ir, damperCage.i)` joins two
arrays of two, and the shape table holds `motor.smpm.ir` and nothing
for `damperCage.i`: the damper cage is a conditional component - `if
useDamperCage` - and a conditional component's shapes are not
measured. The left side knows it is two; the right side, being one
bare name of unknown shape, counts as one.

So this is the same family one storey up: a conditional whose
condition now settles, whose contents are still not measured. Where
the last chain ended in a record's inherited fields, this one starts
at whatever decides not to measure a component that may not exist.

Three synthetic models of that shape - a conditional component with
an array output, connected to an array - flatten correctly, so what
the library does differently is again not yet reduced to a screen.

### The parameter-settling chain, three links in

The advice was three forms, and all three are in: the bodies the run
carries are in view of everything that happens before the run begins
(one map, made once, handed on); the interpreting side of `eval` has
the door for `f(...)[k]` the compiling side already had; and the hard
refusal of the settling round waits until the two queues below it -
what the initialisation claims, what keeps its start - have run, with
one more round over what was left.

The number does not move, and the probe says why: the model waits on
`p_ambient`, which is in none of the three queues. It is a parameter
of an `inner` object - `system.p_ambient = 101325` is right there in
the flat model - and what waits on it is a chain three names long.
So this chain has a fifth link, one storey below anything the answer
described, and it is about how `inner`/`outer` parameters reach the
settling round rather than about bodies at all.

The three forms stay in the tree. They are what makes the rest of the
chain findable: without them the model dies at the first, and each of
the ten places that used to decide on its own that a call it cannot
answer is a call nobody can would have to be found again.

Walked further with the probe, the chain past those three reads:

- **Fifth.** A `NamedArg` reaching the evaluator - the library calls
  its property function with `phase = 0`. Taken: a named argument is
  its value by the time a parameter is settled.
- **Sixth.** `region_pT(p, T)` against a body whose third input
  defaults to zero: the walk counts arguments against inputs and
  refuses. Tried - only the inputs with nothing to fall back on are
  required - and it needs the seventh with it, since the frame then
  lacks the input that was left out.
- **Seventh.** Laying an omitted input out like a local. Tried
  together with the sixth and one test of the suite went red, so both
  are out for now: they need a look at which case that test is
  defending.

So the chain is seven links deep and five are in. The two that are
not are one change, not two, and the test that caught them is the
next thing to read.

### The machine cluster, first trace

Some forty-five models refuse as unbalanced around the induction and
DC machines. Traced on one - `IMC_DOL`, 523 equations for 502
unknowns, 21 too many - the surplus reads:

```text
aimc.is[2] = aimc.plug_sp.pin[3].i
aimc.is[3] = aimc.plug_sp.pin[2].i
```

Two elements of one array assignment, `output SI.Current is[m] =
plug_sp.pin.i`, with their subscripts crossed: the second element of
the left takes the third of the right and the third takes the
second. So the equations are not surplus at all - each pair says the
same thing twice under different names, and the matching has nothing
left for them.

A model of that shape written by hand - a plug of pins, an array
output assigned from `plug.pin.i`, a loop over the pins - flattens
correctly, elements in order. So the crossing comes from something
further in, and the next probe belongs where a member is read off a
run of connectors rather than where the equations are counted.

The message is worth a line of its own: it named these as
`is[2] = its limit`, because the describer prints a bound rather
than an expression for anything that is not a number or a name.
Reading the pair took a probe; it should not have.

### The `statesFM` shapes, and where those thirteen went

Thirteen models refused with `an equation between shapes [1, 2] and
[1, 0]`. The library writes `statesFM = fill(Medium.setState_phX(
...), 0)` where the medium has no trace substances: a run of two
states, each carrying nothing.

The rule that forgives an empty side was comparing every pair of
dimensions, including the last - which asks two to equal nothing.
What has to agree is the dimensions before the empty one: the outer
run is real, the inner is the nothing both sides agree on. Fixed.

The count does not move, and this time the reason is worth reading:
all thirteen walk straight into `heater.h_start asks to be evaluated
before the run`, which is the parameter chain of the section above -
the same `h_start` of the same water, seven links deep with five of
them taken. So this is not a separate family after all. It is a
fifth tributary into the same river, and when that chain is finished
these come with it.

### The parameter chain, eighth link: a constant of a redeclared medium

The thirteen `statesFM` models now stop at `heater.h_start asks to be
evaluated before the run`, and the probe reads that in three steps:

- The binding is already inlined - `if use_T_start then reference_h +
(T_start - 298.15)*cp ...` - so the body did its work.
- Of the three names in it, two are settled: `use_T_start` is 1 and
  `T_start` is 353.15.
- The third, `reference_h`, is settled under no path at all: the
  constants table holds nothing whose name contains it.

`reference_h` is declared without a value in
`Interfaces.PartialLinearFluid` and given one - 104929 - by the
`extends` of `CompressibleLiquids.LinearWater_pT_Ambient`, the medium
this model redeclares into place. So the value exists, in the
library, one `extends` modifier away from the declaration; what has
not happened is its arrival under the instance that asks for it.

A model of that shape by hand - a partial package with a valueless
constant, a package extending it with the value, a component
redeclaring the package and reading the constant in a parameter -
settles correctly, number and all. So again the shape is not it, and
the next probe belongs where a redeclared package's constants are
gathered rather than where they are read.

That probe is in, and it names the ninth link precisely. The
constant is asked for as

```text
Modelica.Media.Interfaces.PartialLinearFluid.reference_h
  from scope Modelica.Media.Interfaces.PartialLinearFluid
```

- of the interface, where it is declared without a value, rather
  than of `CompressibleLiquids.LinearWater_pT_Ambient`, which is what
  the model redeclared into that place and which gives it 104929. The
  gathering side already knows how to read a value out of an
  `extends` modifier; it is being asked about the wrong package.

So the ninth link is: a name written inside a medium's own interface
keeps that interface as its scope when the medium is redeclared, and
the redeclaration never reaches it.

Tried, and worth writing down before it is tried again. The mark that
holds the name a body was reached by - the one that lets a function
of a redeclared medium be found - **is standing** at that moment and
points at the right package:

```text
head = Modelica.Media.Interfaces.PartialLinearFluid
holds = false
asked = Modelica.Media.CompressibleLiquids.LinearWater_pT_Ambient
```

So the walk outwards was taught to ask that package where the
interface says nothing, and the answer cache was given the asked-for
name as part of its key, since the same constant in the same
interface is nothing under one medium and 104929 under another.

The number still does not move. Whatever consumes `reference_h` on
the way to `h_start` is not this walk, or not only this walk - the
model asks the same question through three different bodies
(`BaseProperties`, `setState_phX`, `specificEnthalpy`), and one of
them gets its answer somewhere else. That is the next thing to
measure, and it is where the tenth link is.

### The tenth link, and the two ways it was tried

The advice was exact about the hole: `gather_package_constants`
already takes what an `extends` modifier says about a base's
constant, and the walk outwards then threw that away and asked the
path again, which leads to the bare declaration in the interface. So:
`find` instead of `any`, and work out what was found.

Done, and the model passes it - `h_start` settles, the model walks on
to `tank.medium.Xi`, three storeys further than it has ever reached.
The falsifying probe passes too: two media of one interface, each
giving the same constant a different value through its own `extends`,
settle to 104929 and 209858.

And the library count falls from 733 to 670. Sixty-three models lose
something they had: a value taken from an `extends` modifier now
outranks a nearer declaration that has one of its own.

Narrowed three ways, none of them right yet:

- Ask the path first and use the gathered value only where the path
  says nothing: back to 733, and `h_start` unsettled - the path
  answers, wrongly, rather than saying nothing.
- Use the gathered value only where this package declares the
  constant bare: the probe says `declares = false, gathered = false`
  for `PartialLinearFluid.reference_h`, so neither test fires.
- Ask the medium the body was reached by, where the interface's own
  gathering came back empty: `h_start` settles and the count is 673.

The third is the closest and still costs sixty models. What it means
is that `asked_under` points at the medium in far more places than
this one, and in most of them the interface's answer was the right
one. The distinction that is missing is not "which package" but
"which of the two answers is nearer to the asking" - and that is
where the eleventh link is.

## The register after the constants chain

Taken at 733 flatten and 334 run - the run count having moved for the
first time in nine series.

### What stops flattening at 733

| Count | What it is                                            |
| ----: | ----------------------------------------------------- |
|    27 | a loop whose trip count is not settled                |
|    15 | an argument that must be dimensionless carries a unit |
|    13 | a name with no declaration above it                   |
|    12 | `previous` with no clock across a redeclare boundary  |
|    11 | a subscript outside its array                         |
|    11 | a parameter asking to be evaluated before the run     |
|    10 | a flexible size with nowhere to read a length from    |

The `connect between 2 and 1` family - 21 models, the top of the
last register - is gone entirely. The loops at the top are what the
machines walk into next.

### What stops running at 334

| Count | What it is                               |
| ----: | ---------------------------------------- |
|    18 | an unknown variable                      |
|    11 | a singular Jacobian in an algebraic loop |
|     9 | an unknown function                      |
|     9 | an unbalanced model                      |
|     9 | initialization that is not square        |
|     8 | an algebraic loop that diverged          |

The eleven parameters still asking to be evaluated are the same chain
one storey further along: the constant now settles, and what is built
on it does not yet. Which is the honest state of a ten-link chain
with eight links in.

### The eleventh link: a body's constants arrive without their package

Eleven models still stop at `h_start asks to be evaluated`. The
binding is inlined and reads, in full:

```text
if use_T_start then reference_h + (T_start - 298.15)*cp_const + ...
```

`reference_h` and `cp_const` are bare names. They were written inside
the medium's own function, where the medium is the enclosing package
and a bare name is the right way to say it - but the body has been
inlined into the model, where nothing encloses them. The walk
outwards now answers such a name correctly when it is asked from a
scope inside the medium; here it is asked from the model, and there
is no medium above it any more.

So the constants need to travel with the body: substituted where it
is inlined, in the terms of the package it was taken from, rather
than left as names for a later reader who has lost the package. That
is one storey below everything the chain has taken so far, and it is
where the run count's eleven are waiting.

### The ninth and eleventh links, taken on one road

Both are in, and both only where a parameter's value is being
settled. The walk out through the enclosing packages now asks the
basket of the medium a body was reached by - the `ASKED_AS` mark -
where the package that declared the name answers nothing, which is
what an interface constant like `reference_h` does by design. And
what a body answers with travels with its own package's constants
substituted in, so a bare `cp_const` no longer leaves the medium
behind.

Answering everywhere was measured first and costs twelve models. A
constant carries a unit and the number replacing it does not, so
`h = cp_const*T` reaches the dimensional layer as kelvin against
joules per kilogram and a sound model is refused. A parameter wants
the digit and has no such reader; that is the whole of the gate.

The corpus stands at 733 and 334, unmoved. The proof the branch
works is a redeclare rather than a count: HeatingSystem under
`LinearColdWater` settles `h_start` and walks on to its next
refusal, where before it stopped. The ambient medium's own
`reference_h` is `StandardWater.specificEnthalpy(state)` over a
record constant, which no fixpoint of f64 can hold - that is the
next link, and it is the ladder the last answer named, not this one.

## The register after the loop's road opened

Taken at 740 flatten and 334 run, floors moved with them.

### What stops flattening at 740

| Count | What it is                                            |
| ----: | ----------------------------------------------------- |
|    26 | a parameter asking to be evaluated before the run     |
|    16 | an argument that must be dimensionless carries kelvin |
|    13 | a name with no declaration above it                   |
|    12 | `previous` with no clock across a redeclare boundary  |
|    11 | a subscript outside its array                         |
|    10 | a flexible size with nowhere to read a length from    |
|    10 | a run of elements against a different count of values |

The loops - 27 models, the top of the last register - are gone. What
took their place at the top is the parameter family, and it is the
same chain read from the other end: 26 of them are `X_start[1] =
Medium.X_default` and `h_start = waterBaseProp_pT(...)[1]`, media
constants and standing calls that settle for nobody. The thirteenth
link named in the last letter - a constant array written `fill(e, n)`
that no fixpoint of f64 can build - is what most of them stand on.

### What stops running at 334, the second time of asking

Nothing counts above one: the run half has no family left, only
singles. `cannot evaluate parameters` is every second line of it,
which says the same thing the flatten half says - the wall is
constants that will not fold, not machinery that will not run.

## Working the singles, when the families are gone

The register says the run half has no family left: every barrier
counts one, and `cannot evaluate parameters` is every second line of
it under a different name each time. A method built for families -
take the top of the ranking, fix the kind, count the models - has
nothing left to take the top of. What follows is the form proposed
for the next stage, written down before it is used so that it can be
argued with rather than discovered afterwards.

### Why the old method stops working here

A family is a barrier many models share, so one repair is paid for
once and measured over the whole corpus, and the ranking says which
to take. A single is a barrier one model has. Ranking says nothing;
the corpus cannot measure one repair against the noise of an
eleven-minute run; and the temptation is to take whichever refusal
looks easiest to read, which is how a compiler acquires a hundred
special cases.

### The form: a batch of singles, one chain each, one measurement

- **Five to ten models a shift, chosen by nearness rather than by
  ease.** Nearness means the refusals name the same layer - the
  constants road, the array road, the event machinery - even where
  the wording differs. A batch that shares a layer shares its
  repairs; a batch chosen by how readable the message is shares
  nothing.
- **A probe per model before any repair, and the answer written in
  the batch's note.** What the model asks for, where the asking
  stops, and which layer owns that place. Ten probes cost a minute
  each; ten repairs guessed at cost the rest of the shift.
- **The chain is per model, and it is short by construction.** A
  single is one model's road: two or three links, not thirteen. When
  a link turns out to be shared - two probes stopping in the same
  function - the two models merge into one chain and the batch is
  smaller by one.
- **One corpus measurement for the batch, not one per repair.** The
  library check is eleven minutes; ten of them is a shift. Repairs
  are made against their probes, and the corpus says at the end
  whether anything else moved. A batch that costs models is taken
  apart by re-running the probes, not by bisecting the corpus.
- **A batch that ends with no model moved is still a batch.** Its
  note says which layer each probe stopped in, and three such notes
  naming one layer are a family after all - found by probing rather
  than by counting, which is the only way a family with one member
  per wording can be seen.

### What would say this form is wrong

If two batches in a row end with every probe stopping in a different
layer, the singles are not a stage but a tail, and the honest move is
to stop working them and say so in the register. If a batch's repairs
are each five lines of special case, the same. The form earns its
place by finding shared layers; a form that finds none is a way of
looking busy.

## The constants chain, end to end

Fifteen links, taken over four shifts. Each line is what was wrong
and what it cost or bought; the counts are from `library_floor.sh`
before and after, never from expectation.

| Link | What stood in the way                                                        | State                              |
| ---: | ---------------------------------------------------------------------------- | ---------------------------------- |
|    1 | `programs: None` at the parameter evaluator and nine other places            | taken (0cbbe68)                    |
|    2 | no door for `f(...)[k]` where `f` answers with a record                      | taken                              |
|    3 | a hard refusal of a cycle standing before the claim queues                   | taken                              |
|    4 | `NamedArg` reaching the evaluator                                            | taken (934a28c)                    |
|  5-6 | an input the caller left out, and the arity that must still refuse an excess | taken                              |
|    7 | the shape of an empty dimension, `[1,2]` against `[1,0]`                     | taken (ea6e503), 13 models         |
|    8 | `h_start` waiting on `reference_h`                                           | measured, superseded by 9          |
|    9 | an interface constant answered from the medium on the mark                   | taken, gated to the parameter road |
|   10 | tried three ways and measured; the ladder named instead                      | superseded                         |
|   11 | a body's constants arriving without their package                            | taken, same gate                   |
|   12 | a constant array named whole by a settled branch                             | diagnosed by the panel             |
|   13 | `fill(1/nX, nX)` measured but never built                                    | taken, both constant roads         |
|   14 | a name declared below the interface the body is written in                   | taken                              |
|   15 | a constant an equation reads, which must keep its unit                       | taken, minted as a parameter       |

What the chain bought, in the two numbers the project measures:
flatten went 733 -> 773 and run 334 -> 336 over the same span, but
only three of those steps moved a count. Nine through fifteen moved
none at all: they opened the road that the last of them, the mint,
now walks. The twenty that came at the end came from a leading dot in
an operator's name, not from the chain - which is the honest way to
report it.

The gate is the chain's own rule, and it was measured twice in both
directions: a parameter's road folds a constant to a digit because it
wants the number or nothing; an equation's road takes the name with
its unit, because the digit is dimensionless and the check would
refuse a sound model. Twelve models say so each way.

## The register at 773 and 336

### What stops flattening at 773

| Count | What it is                                            |
| ----: | ----------------------------------------------------- |
|    26 | a parameter asking to be evaluated before the run     |
|    13 | a name with no declaration above it                   |
|    11 | a subscript outside its array                         |
|    10 | a flexible size with nowhere to read a length from    |
|    10 | a run of elements against a different count of values |
|     8 | a record given the wrong number of fields             |
|     8 | a body written in C with no answer here               |

Two families are gone from this half entirely: the dimensionless
argument (16, and the 5 of Spice3 with it) and `previous` with no
clock across a redeclare boundary (12).

### What stops running at 336

| Count | What it is                                |
| ----: | ----------------------------------------- |
|    23 | an unknown variable                       |
|    22 | an unknown variable in an equation        |
|    14 | an algebraic loop that diverged           |
|    11 | a singular Jacobian in an algebraic loop  |
|    11 | `shortPipe.flowModel.dp_nominal` unvalued |
|     9 | an unknown function                       |
|     9 | an unbalanced model                       |

This half has families again, and that is new: the last register
found only singles here. Twenty models past the flattener means
twenty models arriving at the run half together, and the two unknown
variable lines - 45 between them - are the top of the next queue. The
form written above for working singles is not needed yet.

## Batch one of the singles: `cannot evaluate parameters`

Ten models probed before any repair, chosen the way the form says -
by the layer their refusal names rather than by how the message
reads. The note is written whether or not anything moved, and this
time nothing did.

### Where each probe stopped

| Model                              | What nothing gives a value to               |
| ---------------------------------- | ------------------------------------------- |
| `ReferenceAir.DryAir1`             | `Medium.h_default`                          |
| `DrumBoiler`                       | `sink.Medium.h_default`                     |
| `PumpingSystem`                    | `source.Medium.h_default`, and an IF97 call |
| `TestJunctionIdeal`                | `data.R_s`                                  |
| `PressureLoss.Bend`, `.Orifice`    | `data.R_s`, `Medium.h_default`              |
| `TestVolume`, `TestTemperature1/2` | `waterBaseProp_ph(...)[4]`                  |
| `TestSweptVolume`, `Inverse_sh_T`  | a NASA polynomial over `data.alow[...]`     |
| `TestSharpEdgedOrifice`            | `dp_nominal`, arithmetic over `data.zeta1`  |

Ten probes, one layer, three kinds within it - and every one of them
a constant of a medium or of a data record that the flattener reaches
but cannot fold. This is not ten singles. It is one family with ten
spellings, which is exactly what the form was written to find: three
notes naming one layer are a family, and here there are ten.

### What was tried, and why it was put back

The mint of link fifteen already answers this shape, so the question
was why it does not fire here. Two faults found, both real:

- The unit is reached through a chain of aliases -
  `SpecificHeatCapacity = SI.SpecificHeatCapacity`, where only the
  last says a unit - and the walk stopped at the first. Following the
  chain finds `J/(kg.K)` and the mint fires.
- A minted name is born before the prefix pass, so `flat_name` puts
  an instance path on the front of it: `v.medium.Modelica.Media...
cv_const`, a name nothing declares. The panel predicted this
  exactly; the guard it named did not hold, and the reason is not yet
  understood - a second road reaches the name.

With both in, `ThreeTanks` stops refusing and starts compiling - and
takes five minutes instead of four seconds, because the work it now
reaches is work it never did before. That is not a regression to
bisect but a wall moved, and the honest reading is that the batch
needs a shift of its own rather than the tail of this one. Reverted
whole; the tree stands where the register describes it.

### The metric, and what it is worth

`refusals.sh` counts kinds; a probe finds layers. The first batch of
singles put ten models in ten kinds and one layer, so the count of
kinds is a lower bound on the number of families and probing is the
upper one - the gap between them is the work left.

The endgame number, then, is read with that in mind: 247 kinds over
437 models in the run half, 0.57 kinds per model, against 235 over
384 - 0.61 - at the last reading. It fell, and the batch says why
without waiting for it to move: the kinds were never as many as they
counted.

### What this says about the form

The form asked to be told when a batch finds one layer three times
over. It found one layer ten times over, on the first batch, which
means the singles of this half were never singles - they were the
constants chain seen from the run side, under ten different names
because the message quotes whichever parameter happened to be first.
The old method applies: this is a family, and it is taken as one.

## Batch two, and the flatten tails: the rule holds from both sides

The first batch put ten models in one layer and said the count of
kinds undercounts families. This one was chosen to test the other
half of that: ten models of the run half whose refusals name
different layers, and four kinds from the flatten half, probed before
any work was chosen.

### The flatten tails, one probe each

| Kind                                    | Count | Where the probe stopped                                                      |
| --------------------------------------- | ----: | ---------------------------------------------------------------------------- |
| a name with no declaration above it     |    13 | `outer GlobalSeed` unresolved in `PartialNoise` - the inner/outer layer      |
| a subscript outside its array           |    11 | `boundary1.medium.Xi[1]` where `nXi` is 0 - the constants layer              |
| a flexible size                         |    10 | `t_new.columns` sized `2:size(table, 2)` - the table layer                   |
| a run of elements against another count |    10 | `state` of a random generator, `nState` against one value - the arrays layer |

Four kinds, four layers, and one of them - the subscript - is the
constants chain again under a fourth name. So the counter is wrong in
both directions at once: it splits one family into four kinds, and it
also puts four genuinely different layers in four adjacent rows where
nothing says they are unrelated.

### The run half, ten probes

`seedOut[1]` unknown; an algebraic loop diverging on a heating
diode; a singular Jacobian across two MOS heat ports; `previous`
reaching the run as an unknown function; an unbalanced controller
missing two equations; an initialization that is not square; a
Bessel filter's `cr[1]` unvalued; a structurally singular
`kinematicPTP`; a discrete never assigned by any `when`; and one that
compiles and then exceeds the solver's budget.

Ten models, nine layers - the event machinery, the solver, the
matcher, the initializer, the clock layer, the parameter road. These
are singles in earnest, and the form written for them applies here
rather than in the first batch.

### What the two batches settle

The rule stands with evidence on both sides: probing found one family
where the counter showed ten kinds, and nine layers where the counter
showed ten kinds of a different sort. The counter cannot tell those
two situations apart, and nothing in its output ever will - which is
why the probe now comes before the work is chosen rather than after.

### Where the `nXi` family stands, for whoever takes it next

Eleven models say `subscript 1 is outside an array of 0`, and the
probe puts them in the constants layer under a fourth name. Narrowed
to six lines:

```modelica
model NXI2
  package Medium = Modelica.Media.Air.MoistAir(extraPropertiesNames={"CO2"});
  Modelica.Fluid.Sources.FixedBoundary b(nPorts=0, redeclare package Medium = Medium);
  inner Modelica.Fluid.System system;
end NXI2;
```

What is known, all measured on this tree rather than reasoned: the
medium's own constants are right - asked of `MoistAir` under that
modifier, `nXi` is 1, `nS` is 2, `fixedX` is false. Asked from inside
`MoistAir.BaseProperties`, where the refusal happens, they are right
too. And the array `Xi` is nevertheless zero long at that point, so
its length was settled somewhere earlier than the substitution that
gets these numbers - the declaration is `MassFraction Xi[nXi]` in the
interface, and the redeclaring model does not restate it.

So the next link is not the constants road at all but wherever a
declared dimension is measured for a model that a medium redeclares.

### The measurement found, and why it was not taken

The probe went there and named the place exactly:
`measure_dimensions` in `components.rs`, which for `Xi[nXi]` asks
`substitute_class_constants(dimension, ..., scope, ...)` with `scope`
the class that _declares_ the dimension - `PartialMedium.
BaseProperties` - where `nXi` is 0. Every other way of asking gives
1: the medium under its modifier, the medium from inside the
refusing class, `lookup("Medium", ...)` at the site, which resolves
to `MoistAir` correctly.

Three ways of carrying the medium to that measurement were tried and
all three came back None at the point that matters:

- the child's own `effective_imports`, built from `child_redeclares`
  - the imports hold no `Medium` at that depth;
- the package a dotted type name led through, `Medium.BaseProperties`
  - right for the component itself, absent one level down where `Xi`
    is actually measured;
- the `ASKED_AS` mark, held from the component to the end of its
  instantiation - empty at the measurement, so the measurement
  happens on a road that does not pass through it.

That last is the finding worth keeping: the shapes of a redeclared
class are settled somewhere the mark does not reach, which is a
different road from the one every constants link so far has walked.
Reverted whole rather than left half-built. The next probe goes not
to `measure_dimensions` but one level up: which caller settles the
shapes of a component's own components, and what it knows about the
medium when it does.

## Numerical refusals are a queue of their own

`Dimmer_RL` compiles, runs, and stops with `solver exceeded the
evaluation budget at t = 0.000894`. Nothing about it is structural:
the model is whole, the equations match their unknowns, the
initialization is square. What failed is arithmetic - a step the
solver could not take small enough to satisfy its own error test.

Such a refusal does not belong in the same queue as the structural
ones, and putting it there is the same mistake as counting kinds and
calling them families. A structural barrier is repaired by teaching
the compiler something; a numerical one by a solver's tolerance, a
step controller, an event that was missed, or a model that is
genuinely stiff and wants a different method. The evidence that tells
one from the other is different too: a structural refusal is proved
gone when the model compiles, a numerical one only when the run
reaches its stop time with a curve someone has looked at.

The register should therefore carry them apart. Named in the run half
so far: the evaluation budget (1), algebraic loops that diverged (14),
loops that did not converge in fifty Newton iterations (4), and
singular Jacobians (11) - the last of which straddles the line, since
a Jacobian is singular either because the model says so or because
the point it was taken at is unlucky. Thirty models, give or take,
whose repair is arithmetic rather than semantics.

### And one probe when each is filed, because some are ours

A budget exceeded may mean the system is stiff, or it may mean this
compiler is doing work it need not do - a structural fault wearing a
numerical coat. The two are told apart by asking how much wall time
one step costs, not how many steps were taken.

`Dimmer_RL` probed that way: **two steps in 76 seconds**, against a
budget of twenty million evaluations it never came near. Nothing
about that is stiffness. Two steps that cost thirty-eight seconds
apiece are a step function this compiler built badly, and the model
belongs in the structural queue after all - filed under the same
heading as the giants of the performance ledger.

So: when a numerical refusal is filed, probe it once. Steps that are
many and cheap are arithmetic; steps that are few and dear are ours.

## Batch three: the clock layer, five models, one place

Chosen by layer rather than by text, as the form now says. Five
models of the run half whose refusals name the event machinery -
three wordings between them: `previous` reaching the run as an
unknown function, a discrete never assigned by any `when`, and an
unknown `state64[1]`.

The probe put all five in one place, and it is not where the wording
points. In `AssignClock`, `sum.y` _is_ on the clock - the inference
takes it there through the `connect` chain, and the probe prints it
among the clocked names. What is missing is its equation: the
`MathInteger.Sum` block writes

```modelica
if size(u, 1) > 0 then y = k*u; else y = 0; end if;
```

and an equation inside an `if` whose branch is chosen by a size never
joins the partition, so the clocked variable arrives at the run with
nothing assigning it. The refusal is honest and names a symptom two
steps from its cause.

So the link is: an `if` equation whose condition asks a _length_ -
`if size(u, 1) > 0 then y = k*u; else y = 0` - is not settled at all,
because the settling reads constants and the shapes are held
elsewhere. Undecided, it leaves one equation per position choosing
its residual, and the clocked `y` arrives at the run with nothing
assigning it.

### Taken, measured, and put back

The repair is four lines: ask the shapes as well as the constants,
under the instance path, since the condition says `u` and the table
holds `sum.u`. Two of the five models compile with it -
`AssignClock` and `AssignClockVectorized` - and the corpus says 772
flatten and 337 run: **plus one on the run half and minus one on the
flatten half**, which is why it went back.

The minus is honest and the models behind it are the next two links,
both a storey deeper than this one:

- `UpSample` sums two up-samplers of different factors, which is
  legal Modelica and which this compiler refuses as `sum.y is written
on two clocks at once`. It refuses it only now, because before the
  repair the equation never reached the clock inference at all. Two
  clocks of the same family at different rates need the slower to
  enter the faster through the sub-sampling the language already
  spells out.
- `TickBasedSine` refuses as `a continuous equation may only read a
clocked variable through hold` - the same shape of fault at the
  partition boundary.

So the family is one link wide and three deep, and taking the first
without the other two costs a model to win a model. Written down at
the point where the next probe starts: not `measure_dimensions` and
not the `if`, but what the clock inference does with an equation that
names two rates.

### The performance ledger, measured this shift

| Model            | Then                                   | Now        | Where it goes                               |
| ---------------- | -------------------------------------- | ---------- | ------------------------------------------- |
| `ThreeTanks`     | 5 min during the mint, 3.1 s before it | **2.1 s**  | closed - faster than before the chain began |
| `HeatingSystem`  | over 60 s, before any of this work     | **23 s**   | closed by the same three orderings          |
| the library pass | 11 min, 50 min mid-chain               | **11 min** | closed                                      |
| `DoublePendulum` | 45 s                                   | **27 s**   | closed by the array table                   |

The last one probed rather than guessed: a counter on `inline_function`
fires **once** over the whole model. Whatever those forty-four seconds
are, they are not bodies being written out.

### `DoublePendulum`: the array layer, measured

A counter on `expand` says it plainly: **forty million expansions**,
and over the first four million there are **2470 distinct questions**.
One name - `boxBody1.r[1]` - is expanded 126 774 times. It is the
archive's own pattern, a third time: the same value recomputed per
element, here because an orientation built by `from_nxy(r,
widthDirection)` appears in every equation of the body and is walked
whole on each.

A table of what an expression came to takes the model from **44
seconds to 3.1** - fourteenfold, and the largest single win this
project has measured. It is not committed, because the key is not yet
right, and the tests said so rather than the clock:

- keyed on scope and expression alone: two tests red. The mark
  belongs in it - one expression under two media is two answers.
- with the mark, and forgetting on the same beat as a body's answer:
  still two red, and the win falls to 25 s. The shapes in view
  matter too: `sub.suspend[1].reset` against `[2]` is the same
  written expression and a different answer.
- with the shapes of the names the expression itself writes: still
  red at 28 s. So an expansion reads shapes of names it does not
  write - through a record's fields, or a member walked off an
  array - and a key built from the expression alone cannot see them.

That is the finding: the answer depends on more of the environment
than the expression names, and until what that "more" is has been
written down exactly, the table is a way of being fast and wrong.
The next probe is not on the clock but on the dependency - which
parts of `Shapes` an expansion actually reads, measured rather than
assumed.

Reverted whole. The forty-four seconds stand, and now they have a
number and a cause beside them.

### What the three giants had in common

Three giants, three layers, one pattern - and it is worth naming
because it will turn up a fourth time.

| Giant               | Layer             | The repeated work                                                |
| ------------------- | ----------------- | ---------------------------------------------------------------- |
| the constants chain | constants         | a package's whole basket gathered per asking, to answer one name |
| `DoublePendulum`    | arrays            | one expression expanded 126 774 times                            |
| `Dimmer_RL`         | the step function | two steps in 76 seconds, each rebuilding what the last one knew  |

None of them is doing something expensive. Each is doing something
cheap, repeatedly, over a value that did not change between askings -
and each was invisible to every count the project keeps, because a
count of models says nothing about what one model does inside.

The rule that follows, and it is not the same as "cache it": before
reaching for a table, ask what makes two askings _different_. In the
constants layer the answer was the mark and the road, and the table
worked. In the array layer that question is still open, which is
exactly why the fourteenfold win is not in the tree.

### The credit, which is the only number that grows while the rest stand

Worth its own line, since the ledger only ever tracked the total.

| When                        | Models flattened | Pass        | Per model |
| --------------------------- | ---------------: | ----------- | --------- |
| start of the constants saga |              640 | 11 min      | ~1.7 s    |
| after the three orderings   |              773 | 11 min      | 1.9 s     |
| after the array table       |              773 | **5.5 min** | **1.0 s** |

Twenty per cent more work in the same wall time was the first
instalment; halving the flattening half is the second. The debt was
not merely paid off - the compiler now does more work per model in
less time than when it did less, and every part of that came from
removing repeats rather than from doing anything cleverer.

## The clocked pair, taken together and measured

The instruction was to take the `if`-on-a-length together with
`UpSample` rather than reverting the first. Both were written, both
work on their own models, and the pair still costs a model - so it is
written down rather than committed, with the exact line it fails on.

### The second link, as far as it goes

`sum.y` fed by an up-sampler by two and one by three is legal
Modelica: the sum lives on the faster clock, every tick of the slower
being one of the faster's. Written as: among the settled clocks of an
equation, take the fastest, and where every other is a whole multiple
of it starting at the same instant, that is the answer. `UpSample`
and `AssignClock` both compile with it.

What it breaks is a test this project wrote deliberately
(clocks.rs:563): `Clock(1, 10)` beside `Clock(1, 5)` in one equation
must refuse, and 0.2 _is_ a whole multiple of 0.1. Two clocks a model
declared separately are two clocks however their rates compare -
"the slower's ticks are among the faster's" is true of the numbers
and false of the meaning.

Tightened to "and sub-sampling the faster by that factor gives this
one back", using the machinery `same` already has, the test stays red
all the same: the derived clock of an up-sampler and the declared
clock of the same period are equal under `same`, so the test's own
pair passes the tighter gate too.

So the missing distinction is not arithmetic between two clocks but
_where each came from_ - derived by an operator in this equation
against declared elsewhere in the model - and nothing in `ClockSpec`
records it today. That is the third link, and it is a question about
what a clock is rather than about how two are compared. It goes to
the panel, which is where questions of mechanism go.

Measured for the record: the first link alone gives 772 flatten and
337 run - the same minus-one-plus-one as last shift, confirming that
neither half of the pair is worth taking without the third.

## The register at 773 and 336, taken after the array table

The pass is 5.5 minutes now, so this is cheap to take and will be
taken oftener.

### What stops flattening, after the array table

| Count | What it is                                             |
| ----: | ------------------------------------------------------ |
|    26 | a parameter asking to be evaluated before the run      |
|    13 | a name with no declaration above it                    |
|    10 | a flexible size with nowhere to read a length from     |
|    10 | a run of elements against a different count of values  |
|     9 | **a derivative that takes the wrong number of inputs** |
|     8 | a record given the wrong number of fields              |
|     8 | arrays of two lengths that do not fit together         |

The nine in bold are new to this table, and they are the eleven `nXi`
models arrived at their third storey: `saturationPressure_der` is the
derivative of a medium's own function, and the count of its inputs is
read where the medium is not in view. Same chain, same shape, one
floor up - which answers the question of whether there was a third
storey behind `nXi`. There was, and this is it.

### What stops running, after the array table

| Count | What it is                                |
| ----: | ----------------------------------------- |
|    23 | an unknown variable                       |
|    21 | an unknown variable in an equation        |
|    14 | an algebraic loop that diverged           |
|    11 | a singular Jacobian in an algebraic loop  |
|    11 | `shortPipe.flowModel.dp_nominal` unvalued |
|     9 | an unknown function                       |
|     9 | an unbalanced model                       |

Unmoved but for one: the run half is where the singles live and
nothing was worked there this shift.

## What a clock is, answered

The panel confirmed the hypothesis and corrected its letter, which is
worth writing down before any of it is built.

**Identity is structural.** A clock is the constructor that minted it
plus an exact fraction. `subSample(fast, 2)` and a separately
declared `Clock(1, 5)` tick together and are _two clocks_ - the first
is `{Every(0.1), rate 2}` and the second `{Every(0.2), rate 1}`, and
today `same` multiplies the fraction into seconds and bit-compares,
so the trace that is half-present is erased at every gate.

**What is missing is a name, not a chain.** `Root::Every` should hold
a base identifier minted per constructor occurrence, so that two
`Clock(1, 10)` declarations are two clocks. Derivation clones the
root untouched, so the id rides for free. Then `same` compares id,
rate, shift and solver - all exact - and floats leave identity
entirely, keeping the one job they are right for: the `sample(first,
interval)` condition a partition is emitted with. Not the chain:
`subSample(superSample(c, 2), 2)` must equal `c`, which the fraction
already gives and a chain would not.

**Our deliberate test guards the right law on the wrong evidence.**
It refuses a pair whose rates divide, and passes its twin, because
the arithmetic happens to disagree - under structural identity it
would print "one ticking every 0.2 and one ticking every 0.2", which
is absurd. The three laws under it want separating: cross-family
refuses at _every_ factor with a message about being declared apart;
same-family-different-fraction refuses with the crossing advice it
already has; one-clock-two-spellings passes, with `slow` written as
`subSample(fast, 2)` rather than declared.

**And `UpSample` never wanted divisibility.** The rule link two
proposed - fastest wins where the others divide - contradicts a
doctrine this compiler already holds: `work_out` refuses a factor
that merely rounds whole and demands the round trip land home. The
lawful repair is to give the `inferFactor` road a _waiting_ clock and
let the existing inference solve it to equality through the sum. No
new rule at all, and link one's minus-one goes away without link two.

That is the next shift's work, and it is a rewrite of what a clock is
rather than a patch - `ClockSpec`, `same`, `intern`, `canonical`, and
five tests that must change wording to keep guarding what they were
written for.

## The clock rebuilt, and what the `if` still cannot decide

`ClockSpec` now holds a base identity beside its exact fraction, and
`same` compares identity, rate and shift rather than multiplying the
fraction into seconds and bit-comparing. Three laws came out of one
test exactly as the panel set them out, and the corpus did not move:
773 and 336 through a rewrite of what a clock is.

What the rebuild bought is a straight answer where there was a
coincidence. `UpSample` refuses now for a reason that is true - one
base, two fractions, 1/1 against 1/3 - rather than because two
periods failed a bit-comparison.

### Why the `if`-on-a-length still cannot come with it

Taken again on top of the rebuild, and it still costs `UpSample`.
The probe says why, and it is not the clock layer at all:

```modelica
if inferFactor then u_super = superSample(u);
else u_super = superSample(u, factor); end if;
```

`inferFactor` is a parameter, `factor` is a parameter, and the branch
must be chosen by what the _instance_ said - `upSample1` leaves the
factor to be inferred, `upSample2` sets it to three. Settling this
`if` early picks the `else` for both, so `upSample1` super-samples by
its default 1 instead of getting the waiting clock the inference
would have solved.

Narrowing the repair to conditions that actually name `size` does not
help, which is the finding: the branch is still chosen somewhere
earlier than the reading being repaired, so `upSample1.factor` is
read before the instance settles it. The next probe goes there - to
whichever pass decides a parameter-conditioned `if` - and not back to
the clocks.

So the clocked family stands at: identity rebuilt and committed, the
`if`-on-a-length written and measured twice, both times costing the
model that the rebuild was supposed to free. Two independent reasons
have now been ruled out by measurement; the third is a parameter read
too early.

## Every reader of a function's inputs, walked

The derivative's input count was one of nineteen places that filter
`Causality::Input`. The rest were walked rather than waited for, and
the list is here so a twentieth reader can see which shelf it belongs
on.

| Where              | Reads                | Why                                                                                                              |
| ------------------ | -------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `inlining.rs` x4   | the gatherer         | binding a call, seeding a derivative, counting a derivative's inputs, handing shapes over                        |
| `arrays.rs` x4     | the gatherer         | spreading a scalar function over an array, arity of a handed-over function, the replaced input                   |
| `operators.rs` x2  | the gatherer         | which operator function of a record takes this many                                                              |
| `parser/*.rs` x2   | the token            | reading the word `input` off the source, which is where causality comes from                                     |
| `walk.rs` x2       | the class as carried | bodies are carried out with their bases already folded in (`programs_used`), so the run reads what it was handed |
| `components.rs`    | the flat component   | an `input` of a _model_, settled from outside - not a function at all                                            |
| `record_fields.rs` | the declaration      | whether a name a body writes is a field it may reach through, which its own declaration answers                  |

Two of the nineteen were reading a class's own declarations where a
`redeclare function extends` could put them in a base - the arity of
a handed-over function, and which operator function of a record takes
a given number of arguments. Both go through the gatherer now. Nothing
in the corpus moved, and nothing was expected to: like the derivative
before the probe found it, these are wrong only for a medium that
redeclares, and the corpus has few enough of those to hide it.

### And one gatherer, not two

`with_inherited_components` and `function_components` did the same
walk with the same override rule, differing only in how each looked a
base up. The first is now the second under another name, kept for the
readers that mean "a function with whatever it inherits" rather than
"the declaration this call was bound against". Two gatherers is how a
sixteenth reader comes to choose the wrong one.

### One idea kept out of a month-old stash, before dropping it

A branch had sat stashed since the constants saga: _a record-valued
declaration taken apart into one modifier per field_. `Medium()` is a
medium of its defaults; a record given to a whole array spreads over
every element; a field the record declares `final` is not one a value
may hand down. It measured 335 of 734 with five MultiBody models lost
and a test red, and was never finished.

The tree it was written against is a month and forty models old, so
the diff is dead and it is dropped rather than rebased. The idea is
not dead: a record handed to a declaration is still taken as a whole
today, and the register's `function X wants N field(s) for X, got N`
row - eight models - is probably it, seen from the other side. That
row is where to start if it is ever wanted again, and the shape above
is what was already learned about it.

## The flatten layer, cut along its seams

Five pure moves, each its own commit, each measured. Nothing was
improved on the way, which is the whole discipline of it - a move
that also fixes something cannot be read as a move.

| From          | To              | Lines | What went                                                   |
| ------------- | --------------- | ----: | ----------------------------------------------------------- |
| `clocks.rs`   | `machines.rs`   |   741 | state machines, their arrows, what their states say         |
| `clocks.rs`   | `partitions.rs` |   586 | splitting a model into partitions, the counters and markers |
| `arrays.rs`   | `builtins.rs`   |   414 | `transpose`, `cat`, `identity`, the folds over an array     |
| `arrays.rs`   | `shapes.rs`     |   325 | how long each array is, gathered before anything expands    |
| `inlining.rs` | `carried.rs`    |   479 | the bodies a flat model hands to the run to walk            |

`clocks.rs` 2690 to 1387, `arrays.rs` 2833 to 2113, `inlining.rs` 2229
to 1761. The counts did not move once, the register after the sweep is
identical to the one before it line for line, and the pass came out a
second per model faster than it went in - which is measurement noise
rather than a claim.

What each file is for, now that the answer fits in a sentence:
`clocks.rs` is what a clock _is_, `partitions.rs` is what a model made
of them becomes, `machines.rs` is the state machines that ride on
them; `arrays.rs` expands an expression, `shapes.rs` says how long
things are, `builtins.rs` answers the built-ins that build a shape;
`inlining.rs` writes a body out where it is called, `carried.rs`
carries the ones that cannot be.

## The fourth law, found where it lives

The `if`-on-a-length has been measured three times now and each
attempt cost `UpSample`. This shift found where the branch is really
chosen, which is the answer to "earlier than the reading being
repaired": **not in the `if` equation road at all**.

`if inferFactor then u_super = superSample(u); else u_super =
superSample(u, factor); end if` sits inside a `when`, and a `when`'s
`if` is not settled by anyone. `WhenAction::Choice` turns both
branches into a single assignment whose value is `if condition then a
else b` - so `superSample(u, factor)` is worked out even when the
model said to infer the factor, and the block lands on a clock it
never asked for.

Settling that choice where the condition is a compile-time constant
is four lines, and it works: the branch is picked correctly and
`UpSample` walks past it. The wall then moves to `b_super =
superSample(b)`, which is a `superSample` with no factor - a waiting
clock that learns its rate from the equation `y = if b_super <>
previous(b_super) then u_super else 0`, one storey down the chain.

The corpus says 772 and 335 for that alone, so it went back with the
rest. Two findings kept:

- **A `when`'s `if` is never settled.** Every other `if` in the
  language is; this one is not, and no comment says why. It may be
  deliberate - an event's condition may be about the run - but a
  condition made of parameters is not, and the clocked library writes
  several.
- **The inference runs forwards only.** A name whose clock is still
  waiting learns nothing from an equation that reads it; only from one
  that assigns it. `b_super` is assigned by a `superSample` with no
  factor and read by the equation that would settle it, so the two
  never meet. Tried backwards in the same shift and it did not fire,
  because such a name is not in the clock table at all yet - which is
  a third thing to know about it.

Three attempts, three different reasons, all measured. The family is
one link wide and four deep, and this is the fourth.

## Batch four: nine probes, and a family under `unknown variable`

The run half's top row - `unknown variable X`, 23 models - probed
before choosing work, as the form now requires.

| Model                                                        | What is unknown                                  |
| ------------------------------------------------------------ | ------------------------------------------------ |
| `OneTank`, `TwoTanks`, `TankWithEmptyingPipe1`, `EmptyTanks` | `tank.medium.state.T`                            |
| `TanksWithOverflow`                                          | `upperTank.medium.state.p`                       |
| `ShowTransferFunction`                                       | `j` - the imaginary unit, a constant of a record |
| `UnsymmetricalLoad`                                          | `voltageSource1.v[1]`                            |
| `IMC_withLosses`                                             | `combiTable1Ds.y[2]`                             |
| `UniformNoise`                                               | `seedOut[1]`                                     |

Six of nine in one layer, and it is the constants chain again from a
new side: `Medium.ThermodynamicState state` is a record the medium
redeclares - `PartialMedium` keeps it empty and every medium fills it
with `p` and `T` - and the flat model holds the equation
`v.medium.state.T = v.medium.T` with nothing declaring the left side.
The record was expanded under the interface, where it has no fields.

Narrowed to six lines already (`/tmp/hd.mo` in the shift's notes: a
`ClosedVolume` with `ConstantPropertyLiquidWater`), and it is the
sixth face of asked-under - `record_asked_under` exists and is called
from the array layer, but the component pass that turns a record
declaration into its fields does not ask it.

### The address, for the next shift

`PartialMedium.BaseProperties` declares

```modelica
ThermodynamicState state "Thermodynamic state record for optional functions";
```

and the medium's own `BaseProperties` writes `state.T = T; state.p =
p`. The declaration is in the interface, where `ThermodynamicState`
has no fields at all; the equations come from the medium, where it has
two. Nothing refuses - the equations are written and the fields never
declared, so the run meets `v.medium.state.T` with no such variable.

Three probes now, and the negative ones did the narrowing.

The component pass never sees this declaration: neither
`component.name == "state"` nor a type naming `ThermodynamicState`
fires there. A print inside `record_fields_of` - the one function all
nine callers use - names who _does_ ask about that record, and the
answer is instructive:

| Asked by                                     | Found                                              |
| -------------------------------------------- | -------------------------------------------------- |
| `record_fields::record_input_fields`         | `PartialSimpleMedium.ThermodynamicState`           |
| `arrays::written_out`, `records_written_out` | the same                                           |
| `arrays::expand`                             | `PartialMedium.ThermodynamicState` - the empty one |

So the machinery that asks under the medium works and finds the
record with two fields; the array layer's own expansion asks under the
interface and finds the empty one. Three of four callers are right and
the fourth is the wall - which is a narrower target than "somewhere in
the flattener", and it is where the next probe goes.

### Two repairs tried at the fourth caller, both measured

The array layer's expansion of a record-named `Ref` was the wall, and
asking `record_asked_under` there changes nothing: a print says the
table already holds `PartialMedium.ThermodynamicState`, so the wrong
record was written down before the expansion ever read it.

Following that back, the table is filled by `collect_records`
(scoping.rs), which resolves each declaration's type in the scope of
the class that wrote it - the interface - and stores what it finds.
Asking the mark _there_ changes nothing either: `collect_records` runs
before instantiation, and the mark is pushed by instantiation.

So the wall is neither the reader nor the writer of that table but its
timing: the record paths of a class are gathered once, in the terms of
the class that declared them, and a redeclaration that arrives later
cannot reach them. That is a different shape of fault from every other
face of asked-under so far - those were all a name asked in the wrong
scope, and this is a name asked at the wrong time.

Which is the finding, and it is worth more than the two failed
repairs: the fix is either to gather the paths later, once the
medium is known, or to re-ask them at expansion time from the type
name rather than trusting the table. Both are a shift's work and
neither should be started at the end of one.

That is the next link, and it is a family of at least six by this
probe alone. The other three are genuinely separate: a record's own
constant, a connector's array element, a table's second column.

### The three that went their own way

The batch's other three probes, each in its own layer, which is what
the form was written to find:

- **`ShowTransferFunction`**: `j`, the imaginary unit -
  `final constant Complex j = Complex(0, 1)` in `ComplexMath`, reached
  by `import Modelica.ComplexMath.j` and written into an equation
  whole. The constants layer folds a record constant by its own
  constructor where the name resolves, and an _imported_ name resolves
  elsewhere; the flat model keeps `... * j` with `j` undeclared. Near
  the record-state wall in kind - a record that never became fields -
  and reached by an import rather than a redeclaration.
- **`UnsymmetricalLoad`**: `voltageSource1.v[1]`, an element of a
  connector array.
- **`IMC_withLosses`**: `combiTable1Ds.y[2]`, the second column of a
  table.

Three layers, three walls, and no reason yet to think any two are the
same. These are the singles the method was built for, and they are
the first genuine ones the run half has offered.

## What the specification says about inferring a factor

The panel read chapter 16 and the answer changes the shape of the
work. Written down before any of it is built.

**There is no prescribed order and no fixpoint in the letter.** 16.7.2
gives base-clock partitioning as literal steps, and they exist to make
_the partitions_ well-defined rather than to sequence a compiler.
16.5 says of an absent factor only that it is inferred. 16.7.5 states
a demand on the _result_: within a base-clock partition every
sub-partition's factor and shift must be determined and consistent, or
the model is erroneous.

Which answers the fear inside the question. A fixpoint of **forced**
steps - each drawing only what the constraints compel, which is what
`work_out` already does with its solve-then-prove-by-re-derivation
law - computes exactly the determined set and nothing more. It cannot
settle what the language leaves undefined, because that would take a
guess. And this tree has already made that guess once: the unpicked
`else` branch's default factor is where an inferring up-sampler gets a
clock it never asked for.

**The unit is the base-clock partition**, not the equation and not the
model: `sample` and `hold` are boundaries the inference must not
cross, and our own test that a value read only through `hold` stays
free is the proof.

**But the connected component need not be built.** Iterating over
equations as undirected constraints closes over it already. Three
smaller things are missing instead:

1. The reverse feed: an equation whose target is clocked should push
   that clock into the constraint set and let `work_out` settle the
   waiting rows against it - the one-step machinery already exists.
2. Representability, which comes for free once the joiner runs
   interleaved with the forward loop, since waiting rows re-mint each
   pass.
3. The joiner as it stands is a joiner and not an inferrer: it copies
   a row index and never calls `work_out`, so a conversion's target
   can land on a reader's clock with the whole-number law skipped.

**And a bare factor can be genuinely free.** Refusal stays, but only
for two of three cases: free (read only through `hold` - the words
should say so) and constrained-but-unsatisfiable (the existing
messages, all correct). The third - constrained through a reader - is
where today's message is simply false, and it is the one the fixpoint
makes compile.

The four lines that settle a `when`'s `if` are confirmed right and go
in with this, not before it: alone they let the default factor be
guessed, which is the phantom behind the whole family.

## The register at 782 and 341, taken after the clock series

Taken with `refusals.sh both`, and the third use of the instrument
paid off at once: it caught a family moving rather than a barrier
falling.

### What stops flattening, after the clock series

| Count | What it is                                            |
| ----: | ----------------------------------------------------- |
|    26 | a parameter asking to be evaluated before the run     |
|    17 | **arrays of two lengths that do not fit together**    |
|    13 | a name with no declaration above it                   |
|    10 | a flexible size with nowhere to read a length from    |
|    10 | a run of elements against a different count of values |
|     8 | a record given the wrong number of fields             |
|     8 | a function written in C                               |

The nine derivatives that took the wrong number of inputs are
**gone**, the whole kind reporting zero. They did not disperse: the
array kind
went 8 -> 17, and the nine new ones are the `nXi` models by name
(`Media.Examples.MoistAir`, `Fluid.Examples.BranchingDynamicPipes`,
`TraceSubstances.RoomCO2`, and so on). Same family, one floor up
again, and this is the register earning its keep: a count alone would
have read as nine fixed and nine broken.

### The fourth storey of `nXi`, probed

The diagnosis written at the third storey does **not** reproduce. The
`ASKED_AS` mark is present at the measurement and holds
`Modelica.Media.Air.MoistAir`; `nXi` under the declaring class comes
to 1, correctly; and both arrays measure right - `medium.Xi` is `[1]`
and `medium.X` is `[2]`. Nothing is wrong with the shapes.

Where it actually stops is inside a function body. `MoistAir`
redeclares

```modelica
state := if size(X, 1) == nX then ThermodynamicState(p, T, X)
         else ThermodynamicState(p, T, cat(1, X, {1 - sum(X)}));
```

The branches are of different lengths, so the condition has to be
settled while the body is worked out. At that point the body's tables
hold neither: `sizes["X"]` is absent and `consts["nX"]` is absent. The
argument is the package constant `X_default[nX]`, whose shape never
reaches the shape table because it is a constant rather than a
component of the model.

So the barrier is of the **time** kind, not the place kind: the length
is known, and is not known _there_. The next probe goes to what a
function body is handed, not to what a medium resolves to.

### What stops running, after the clock series

Unmoved: unknown variables (24 and 21), algebraic loops that diverged
(14), singular Jacobians (11), `shortPipe.flowModel.dp_nominal` (11).
Nothing was worked in the run half this series.

## The one model the machines disagree on, named

The build machine runs 342 and this desk runs 341. The difference is
one model, and it is
`Modelica.Electrical.PowerConverters.Examples.ACAC.Dimmer_RL`.

It is the model already filed under the evaluation budget: two steps
in seventy-six seconds, against a budget of twenty million
evaluations it never approaches. Nothing about it is stiff. Whether
it finishes therefore depends on how fast the machine is against the
wall clock the budget is measured beside - which is exactly why the
floors are set from the lower of the two counts.

The hypothesis is the solver budget, not floating-point order and not
thread count: the same model is the only entry under that heading,
and the two counts differ by exactly it.

## A measuring pipe must not cut its own output short

The floor script measured 782 and 341, printed them, and exited 1.
Nothing was wrong with the numbers or the compiler.

`echo "$report" | head -1` is the whole of it. `head` closes the pipe
once it has its line, `echo` is killed by SIGPIPE for writing into a
closed one, and `pipefail` reports the pipeline as failed. On a desk
the report fits the pipe buffer and `echo` is finished before `head`
leaves, so it never fires; on the build machine the list of models
that ran does not fit, so it fires every time.

This is the third way a measuring pipe has lied about its result,
after the script that did not exist and the zero that came from
nothing. The rule grows a clause: a measuring pipe does not swallow
stderr, does not answer nothing where nothing ran, and does not cut
its own output short. Where one line is wanted, take it without a
pipe - `printf '%s\n' "${report%%$'\n'*}"`.

## The `dp_nominal` family: a barrier that fell without the models moving

Eleven models were refused `parameter <pipe>.flowModel.dp_nominal has
no value`, one name across the whole list, which the instrument flags
as a family rather than eleven singles. The probe bore that out.

`dp_nominal` is a parameter of a pipe's flow model, and a pipe sets it
by a class-level redeclaration:
`redeclare model FlowModel = NominalTurbulentPipeFlow(dp_nominal =
1e5)`. That modifier both replaces the replaceable model and gives one
of the replacement's parameters a value. The alias `FlowModel` is a
pair of names with nowhere to hold the modifier, so it was set aside
by the resolved type's name - through `remember_filled_inputs`, the
same store a function's partial application uses - and then read only
where a function body is worked out. A component typed by the alias is
a model, not a function, so its parameter never saw the value.

The repair reads what a class-level redeclaration set aside for a
component's resolved type and folds it in as a modifier, at the lowest
precedence, and only for a non-function type. The kind went from 11 to
0, measured with `refusals.sh`.

But the eleven did not cross to running. `InverseParameterization`,
freed of `dp_nominal`, now stops on the IF97 water functions
(`waterBaseProp_pT`, `visc_dTp`, `dgesv`) that the parameter
initialisation cannot evaluate. This is a barrier falling without the
models moving: the family is gone from its column, the flatten count
is unchanged, and both are true at once. The honest reading is that
the `dp_nominal` wall stood in front of a deeper one, and only the
census - not the totals - shows the first fell.

## The numerical queue, kept apart from the structural one

The run half carries two kinds of refusal that must not be counted
together, because their repair and their proof are different.

A **structural** refusal is repaired by teaching the compiler
something, and proved gone when the model compiles or runs: an unknown
variable, an unbalanced model, a singular structure.

A **numerical** refusal is repaired by a solver's tolerance, a step
controller, a missed event or a stiffer method, and proved gone only
when the run reaches its stop time with a curve someone has looked at.
`Dimmer_RL` is the type: it compiles, runs, and stops at the
evaluation budget, and nothing about it is structural.

Named in the run half so far, numerical: the evaluation budget (1),
algebraic loops that diverged (14), loops that did not converge in
fifty Newton iterations (4), and singular Jacobians (11, straddling
the line - a Jacobian is singular either because the model says so or
because the point it was taken at is unlucky). Thirty-odd models whose
repair is arithmetic, not semantics. They are not worked in the same
pass as the structural queue and should not be read in the same
column.

## The three run-half singletons, probed

The method's form for a single: probe the layer before choosing the
work. All three were probed; none is the clean two-or-three-link
single the register hoped for, and the probes say why.

### `IMC_withLosses` - the table's second column, a four-link ordering chain

`combiTable1Ds.y[2]` is refused `unknown variable`. The probe followed
it to the root, narrowed to five lines:

```modelica
partial block SIMO parameter Integer nout = 1; RealOutput y[nout]; end SIMO;
block CombiTable1Ds
  extends SIMO(final nout = size(columns, 1));
  parameter Real table[:, :];
  parameter Integer columns[:] = 2:size(table, 2);
end CombiTable1Ds;
```

The chain is `y[nout] <- nout = size(columns, 1) <- columns =
2:size(table, 2) <- table`, four links. The probe on `measure_dimensions`
caught the exact failure: when the inherited `y[nout]` is measured,
`local_consts["nout"]` is absent, `columns` is absent, and `table`'s
shape is absent - none of the chain has settled - so `off_a_length`
falls back to the base default `nout = 1`. `y` is fixed at length one,
and `y[2]` is outside it. The parameter `nout` does settle to 4 later,
but `y` is measured once and never re-measured.

This is a **time** barrier, not a place one: the length is knowable
and is measured too early. The register's older note about `nout` was
right that it settles to 4; what it missed is that `y` was sized before
it did. The repair is fixpoint-ordering - defer measuring an inherited
array whose dimension parameter an `extends` overrides until that
parameter's own chain settles - which is a pass-order change, not a
line. A precedence tweak in the settling loop was tried and measured
inert: the simple constant override (`extends SIMO(final nout = 3)`)
already works, and the size-dependent chain needs the re-measure, not
a better binding. Reverted rather than committed, since a change that
moves nothing is not a change.

### `UnsymmetricalLoad` - a complex read as a scalar, the record-state layer

`voltageSource1.v[1]` is refused `unknown variable`. `v` is a
`ComplexVoltage[m]` and the apparent-power equation reads
`S[m] = {ComplexMath.abs(v[k]*conj(i[k])) for k in 1:m}`. The probe
showed `S[1]` expanded to `sqrt((v[1]*i[1].re)^2 + ...)` with `v[1]`
left bare - the `ComplexMath.abs` of a complex product was inlined but
its record argument `v[1]` was never broken into `.re`/`.im`. This is
the record-state wall - a record that never became fields - one house
in from the imaginary-unit `j`, and the same layer, not a single of
its own.

### `ShowTransferFunction` - the imaginary unit, unchanged

`j`, imported as `Modelica.ComplexMath.j` and written into an equation
whole. Named in the earlier register entry and unmoved: the constants
layer folds a record constant by its constructor where the name
resolves, and an imported name resolves elsewhere. Same record-state
layer as `UnsymmetricalLoad`, reached by an import rather than a
connector read.

Two of the three are the record-state layer, one is fixpoint-ordering.
None is the isolated single the run half was hoped to be offering; the
run half's walls are families the flattener's families stand in front
of, one storey down.

## Four barriers of time, gathered - and what building the twin found

The register carried four refusals under one wording, "length known,
measured too early": the fourth storey of nXi (17), the table's second
column (`IMC_withLosses`), the 26-now-27 parameters asked before the
run, and the constant chain behind `434eb3c`. Each shift had patched
one, measured it inert or half, reverted. This shift gathered all four
into one question for the panel, with the pipeline map, rather than
patch a fifth.

### The pipeline, as it measures length

For one class, `instantiate` runs: `measure_shapes` (into
`collect_shapes_given`, handed the overrides) before any component is
built, then `collect_records`, then `instantiate_components` - a
per-component loop with a small parameter fixpoint at its head that
also calls `measure_dimensions` per component. Two length-measurers,
the fixpoint between them.

### What the four turned out to be, probed

- **A (nXi, 17) and C-fluid (25 of 27)** are one thing on two roads: a
  package constant that does not reach a body worked under the
  interface's scope, as a length in A (`X_default` for `size(X, 1) ==
nX`) and as a value in C-fluid (`reference_h`, `cp_const` in an
  `h_start` binding). The 434eb3c hop settled the isolated case; the
  scope is the storey behind it.
- **B (the table)** is genuinely pipeline-order: `y[nout]` is measured
  while `nout = size(columns, 1) <- columns = 2:size(table, 2)` is
  unsettled, falls to the base default `nout = 1`, and `y` is never
  re-measured. A four-link chain, its own fixpoint-input fault.
- **The 2 MultiBody of C** (`lengthDirection[3] = r[3] - r_shape[3]`
  under `normalize`) are a value fixpoint, a third kind.

So the "measured too early" wording fused three causes: a missing road
(A + C-fluid), a pipeline order (B), and a value fixpoint (2). The map
went to the panel with that reading offered for correction.

### The array twin, built and measured against a third storey

While the answer cooked I built the array twin the fortieth shift
named - `asked_as_constant_array`, the mirror of the scalar
`asked_as_constant` - so a constant array asked from inside the
interface is asked again of the medium the mark names. It fires and
reaches the medium: `X_default` under `MoistAir` now resolves to
`{0.01, 0.99}` at the call site, shape `[2]`, where before it was a
bare `Ref` shape `[]`. The road is right.

But the model failed worse, `lens 2 3` where it read `1 2`, and the
cause is a storey below the twin. `MoistAir(extraPropertiesNames =
{"CO2"})` has `nX = 3`, but its `reference_X` is declared `{0.01,
0.99}` - length 2 - and our compiler does not lengthen it by the extra
property. The twin faithfully returns the declared constant, now the
_wrong length_ for this medium. So the twin does not merely fail to
help - it produces a value of the wrong length, and it was reverted
whole rather than shipped.

The finding sharpens A: it is not one storey (scope) but two. The
scope twin reaches the medium; a second thing must lengthen a medium's
own `reference_X`/`X_default` by its `extraPropertiesNames`, upstream
of every reader. If the medium's constants are the wrong length at the
source, neither the scope twin nor a reader-side re-measure helps. The
architecture question went to the panel with this storey on the table.

### The record-state family is wider than three, and mixed

Probed as a family. The census caught about seventeen models, but of
several sub-kinds: ten-plus QuasiStatic models refused `an equation
between shapes [2] and [3, 2]` (a complex vector against a matrix),
`ShowTransferFunction` on the imaginary unit `j`, two `ComplexMath`
tests on `an array cannot be a divisor` (complex `./`). The bare-scalar
case (`voltageSource1.v[1] * conj(i[1])` read as scalar) probed to a
precise mechanism: `record_class_of` does not see an element of an
array of records - `v[1]` of `C v[m]` - as a record operand, so the
overloaded `*` is not applied and `re_of(...)` is dropped. A narrow
fix that resolved the array-element case over-triggered and broke a
record-zero test, so it was reverted; the family wants a careful
reading of where an array-of-records element is and is not a record,
not a one-line widening. Recorded as a family with named sub-kinds for
the next shift, which should take the layer, not a symptom.

## The four barriers, worked: three repairs and a family still standing

The panel ruled the forty-first shift's architecture question: no
general re-measure pass, because by the time a chain could settle the
things written against it are already built, and retracting them is
machinery that does not exist. Three separate works, three prices -
and the ruling overturned this register's own addendum, with four
witnesses. Nothing is declared short; `nX` is 2 for `MoistAir` under
`extraPropertiesNames`, `extraPropertiesNames` counts `nC` beside `X`
and never inside it, and the twin had returned the right length all
along.

### The second storey, taken first: a package alias hands a constant

`package Medium = MoistAir(extraPropertiesNames = {"CO2"})` names a
medium and gives one of its constants a value in the same breath. The
alias had nowhere to hold the modifier, so it was set aside under the
resolved name - the store a redeclaration uses for a function's filled
inputs - and never read where the package's own constants are
gathered. `nC = size(extraPropertiesNames, 1)` counted the interface's
empty default: nought, however many were written.

The two statements are now kept apart rather than sharing one store,
because they are not the same statement. Mixing them had made
`redeclare package Medium = Oil(rho = 3)` overrule the `rho` Oil
declares for itself. Measured: `nC` is 1 for one extra property and 2
for two, and the four numbers a medium is asked for read 2, 2, 1, 1 -
exactly what the panel predicted.

### The first storey: a medium's count reaches a body

The twin reinstated whole, with the complement the ruling named. The
scope walk's body is lifted into `constant_array_of_package` so it can
be asked of a package that is not on the walk;
`asked_as_constant_array` asks it of the medium the mark names. And
the numeric road's gate is asked of the declaration rather than the
road: it answered only while a parameter was being settled, because
the reason to hold a medium's constant back is the unit it carries,
and a count has no unit and no dimensional reader.

Measured on the minimal case: a medium with `nX = 2` whose body picks
`if size(x, 1) == nX` answered 0 before - the else branch, the count
read at the interface as one - and answers 0.5 now. Inside `MoistAir`
the condition reads `1 == 2` and `2 == 2` where it read
`size(X, 1) == nX` unresolved. The seventeen do not move: a residual
zip still pairs two lengths one storey past the condition.

### Barrier B, counted first and then taken

Counted with the instrument before building, as the shift was told:
two models, both `IMC_withLosses` - the Electrical and the Magnetic -
on one name. Under a dozen, so a pointed repair rather than a pass.

Two things were missing at one place, `instantiate_bases`. The lengths
of the extending class did not travel with a modifier handed to a
base, so `table = {{Ptable[j], ...} for j in 1:size(Ptable, 1)}` could
not be built - `Ptable` is the model's parameter and the base has
never heard the name. And the reading that builds a value to find out
how long it is did not exist, so a handed value that is neither
written out nor a range said nothing. With both, `table` measures
`[14, 5]`, `columns` counts four, `nout` is four, and `y[4]` is the
last output rather than outside the array.

Both models of the family moved, from `unknown variable
combiTable1Ds.y[2]` to an unbalanced count: the barrier gone, a deeper
wall behind it. Flattening 1014ms a model against 995 before, inside
the noise - the build is asked last, after every cheaper reading has
failed.

### The record-state sub-family, probed and left standing

`BalancingStar` and its nine neighbours refuse `an equation between
shapes [2] and [3, 2]`. The source is `y = k*uInternal` in
`ComplexBlocks.ComplexMath.Sum` - the scalar product of two complex
vectors, which our compiler spreads element by element into a
three-by-two instead of summing to one complex. The probe narrowed it
that far and no further: the multiplication never reaches `combine`,
so something above it claims the expression first, and the shape
`[3, 2]` - three records of two fields - is not the `(1, 1)` the
scalar-product arm asks for. Left standing rather than half-fixed;
the next shift takes it with that much already known.

## The register at 793 and 341, after the value fixpoint and the scalar product

Two works, both of the kinds the panel separated out, and the first
numbers to move in four shifts.

### The MultiBody value fixpoint

A parameter is settled while its own class is instantiated, against
the values known by then, and a binding may name a parameter of a
component built afterwards: `FixedTranslation r = {0, -1.6,
wheel.rTire}` is written above the wheel it reads. The probe found
`wheel.rTire` worth 0.25 in the table all along - the neighbour had
settled in its turn, and nothing went back to ask again, so `r[3]`
stayed unknown and the direction taken off it, with `Evaluate = true`
on it, was refused.

One more round now runs over the model's parameters once every
component is built, until nothing new comes of it. Only bindings, only
where the name is still without a value. `Surfaces` moves off it;
`PlanarLoops_analytic` stays and rightly - its `n_b` is `fixed =
false` with an equation behind it, which the initialisation solves
rather than the compiler. That is the two the panel counted: one was
ours, one was never ours.

783 flatten from 782.

### The complex scalar product

`y = k*u` of two `Complex[3]` is one complex number, the way
`Real[3]` times `Real[3]` is one real. An operator written for one
record and handed arrays of them was vectorized - one multiplication
per element, right for `+` and `-` and wrong for `*` - so the complex
`Sum` block answered with three values where it means one, and the
equation between one record and three was refused as a shape
mismatch. The pairs are multiplied with the record's own `*` and
summed with its own `+`.

793 flatten from 783: ten models at once, the largest single move in
many shifts.

### What the instrument caught, taken after the series

`refusals.sh both`, and two families have left the head of the list.

| Kind                                      | Before | Now |
| ----------------------------------------- | -----: | --: |
| arrays of two lengths that do not fit     |     17 |   3 |
| an equation between shapes [2] and [3, 2] |     10 |   1 |
| a parameter asked before the run          |     27 |  25 |

The array kind fell from seventeen to three, which is the twin and the
medium's own count working through the media models. The complex
shape kind fell from ten to one. The before-run kind lost the two
MultiBody entries and kept its twenty-five fluid ones, which are the
`reference_h` road and not this shift's work.

The run half moved the other way, and honestly: `unknown variable in
equation` went 21 to 41, because ten QuasiStatic models that used to
stop at flattening now get past it and stop at the next wall - the
neighbouring sub-family, a complex read as a scalar. A barrier
removed upstream shows as a barrier grown downstream, and the totals
say which of the two happened: 793 flatten against 783.

## Who the second ten were: nobody new

The register asked a fair question of the instrument rather than of
memory: `unknown variable in equation` went 21 to 41, twenty models,
and only ten QuasiStatic had been pushed past flattening. Who were
the other ten?

First, the arithmetic was wrong, and the instrument said so. There
are two kinds with that wording, not one. Before: 24 plain plus 21 in
equation, forty-five in all. After: 7 plain plus 41 in equation,
forty-eight. The growth is three, not twenty - the rest is one kind
draining into the other as models travel. A count read off one line
of the census would have sent a whole shift chasing seventeen models
that never existed.

### What the forty-one actually are, by name

| Count | Name                               |
| ----: | ---------------------------------- |
|    14 | `medium.state.T`                   |
|    12 | `fluidConstants[N].molarMass`      |
|     5 | `v[N]` (the complex-as-a-scalar)   |
|     3 | `Air_Utilities.Basic.Constants.MM` |

The two largest are one road, which the probe settled by following
both to the same wall: `SimpleLiquidWater` stops at `medium.state.T`,
and `MediaTestModels.Air.SimpleAir` stops at
`volume.medium.state.T`. Same name, same layer.

### The road, probed to its root

`PartialMedium.ThermodynamicState` is declared **with no fields at
all**, and the library's own comment beside it says why: "in the base
class since the ThermodynamicState record is still empty". Every
medium redeclares it with the pressure and temperature its state
really is - `PartialSimpleMedium.ThermodynamicState` holds `p` and
`T`.

`BaseProperties` declares `ThermodynamicState state` by the plain
name. `collect_records` resolves that name where the declaration was
written, so it finds the empty base: the probe shows `medium.state`
resolving to `PartialMedium.ThermodynamicState` (fields: none) while
the model's own `state` resolves to
`PartialSimpleMedium.ThermodynamicState` (fields: `p`, `T`). With no
fields there is nothing to expand, so `medium.state.T` is a name of
nothing while an equation still writes it.

This is the sixth face of asked-under, on the records road: the name
is right, the scope is wrong, and the medium that would answer is not
in view because `collect_records` runs before instantiation and walks
into `BaseProperties` from the declaring class rather than from the
site. A first attempt to resolve the name again under the site's own
imports was measured inert - inside `BaseProperties` there is no
`Medium` alias any more, the walk having already descended - and
reverted. The medium has to be carried in, not looked up again, which
is the same shape the `asked_as` mark solved for constants and
functions.

Twenty-six models stand on it, and they are the largest single family
left in either half.

### A note on the commit before this one

Its message reads with holes in it - `medium.state.T`,
`fluidConstants[N].molarMass`, `PartialMedium.ThermodynamicState`,
`BaseProperties` and `collect_records` are missing from the prose.
The heredoc that carried the message was unquoted, so the shell ran
every backquoted name as a command and put its empty output in place
of the name. The register above holds the same account with the names
intact, which is why the loss is a blemish rather than a gap.

Not amended: the commit was already pushed, and the branch refuses a
force - correctly. A history that can be rewritten is a history
nobody can trust, and a message with holes is a smaller price than
that.

The rule the performance ledger already carries about pipes grows a
fourth clause, since this is the same family of fault: a measuring
pipe does not swallow stderr, does not answer nothing where nothing
ran, does not cut its own output short - and text carrying names is
quoted at the boundary it crosses. `<<'EOF'` rather than `<<EOF`,
every time a name with backquotes in it goes into a message.

## The array kind's remnant, probed: not what the count suggested

Three models keep the array kind alive, and the fourth item of the
shift was to finish them off - a kind emptied to nought is cleaner
than a kind halved. The probe says they are not the cheap remainder
they looked like.

The pair that fails is the fingerprint of the two branches again:
`{0.01, 0.99}` of length two against `{0.01, 0.99, 1 - sum(...)}` of
length three, the `then` and the `cat` of `setState_pTX`. But the
condition is settled - a probe at the assignment shows it reading
`size({0.01, 0.99}, 1) == 2` with both sides plain numbers - and no
`Expr::If` in the array layer ever sees it: neither the branch-picking
path nor the in-a-loop path fires for this pair. The zip is reached
from `one_assignment` through `expand`, so the two lengths are being
paired by an assignment inside the body rather than by the `if` that
was supposed to choose between them.

Which means the branch is chosen correctly and something downstream
still holds both answers. Probed that far, reverted whole, and left
for the next shift with the trail written down: the fault is one
expression past the condition, in what the body does with the state
it just built, not in the condition or the constants that feed it.

Two shifts have now ended at this same door from different sides -
once from the constants road, once from the array road - and both
times the storey behind it was the same body. It is a body-level
question, and the next attempt should start inside `setState_pTX`
rather than at either road that leads to it.

## Complex read as a scalar, probed to the line - and why the narrow fix is not narrow

Ten QuasiStatic models stand on `unknown variable v[1]`. The
quasi-static library takes a power with
`P[m] = {ComplexMath.real(v[k]*conj(i[k])) for k in 1:m}`, and the
compiler makes `P[1] = v[1] * i[1].re` of it - a complex times a
field, with `real` and `conj` both gone astray.

The probe named the line. At the inlining of `real`, the argument
arrives already wrong: `conj(i[1])` is handed its record correctly,
but the multiplication `v[1] * conj(i[1])` has by then collapsed into
arithmetic on names. A probe on the operator dispatch says why in one
word: `record_class_of(v[1] * conj(i[1]))` answers `None`. The tables
key an array of records by its bare name, `v`, so the element `v[1]`

- the name flattening itself writes - is not known to be a record,
  the operator written on `Complex` never applies, and `*` falls
  through to arithmetic between two names that each stand for two
  fields.

### Two attempts, both measured and both reverted

**Widening `record_class_of`** to read `v[1]` as the record `v` holds.
It answers the operator question, and it breaks
`a_record_may_say_what_its_zero_is`: `sum(arr)` over an array of
records begins to build its zero from the wrong parts, because that
function answers every question about records and not only the
operator's.

**A separate `operand_record_of`**, asked only at the operator
dispatch, strictly - the base must be both an array of known shape
and a record of known class. The same test still fails, which says
what the first attempt did not: `sum` reaches the operator dispatch
too. The narrow fix is not narrow because the door is shared.

So the layer is named exactly - an element of an array of records is
not seen as a record where an operator is chosen - and the repair
needs the one thing neither attempt had: a way to tell an operator
asking about its operand from a builtin asking about its argument.
Both attempts reverted whole; the corpus is untouched and the trail
is written down for whoever holds it next.

## The map of C-fluid: not a road at all, but a function nobody runs

Twenty-five models, the largest family in the flatten half, and the
one the register kept calling "a different road". The map, probed end
to end, says it is not a road in the sense the other three were.

`heater.h_start` is bound to an expression naming `reference_h` and
`cp_const`, medium constants of `PartialLinearFluid` declared with no
value - the medium supplies them. So far this is the nXi shape, and
the probe checked that first: **the mark is present and correct**. It
holds `CompressibleLiquids.LinearWater_pT_Ambient`, the medium the
model named, on all ninety-four askings.

Where it parts company is one line further. Asked of that medium, the
constant is `None` - because the medium does not hold it either. It
extends `Common.LinearWater_pT`, which extends `PartialLinearFluid`
with

```modelica
reference_h = Modelica.Media.Water.StandardWater.specificEnthalpy(state),
cp_const = Modelica.Media.Water.StandardWater.specificHeatCapacityCp(state),
```

The constant is not a number written anywhere. It is **a call into
the IF97 steam tables**, and the refusal of a second model says so in
plain words: `pipe1.h_start` is bound to
`Index(Call("Modelica.Media.Water.IF97_Utilities...`.

### Why this is a different kind of work

The three causes the panel separated were about a value or a length
not reaching a place. Here the value reaches nothing because it does
not exist yet: it is the result of running a large numerical function
before the simulation starts, and that function is the IF97
formulation - a chain of correlations over regions of the
pressure-temperature plane.

So C-fluid is not a scope question, not an order question and not a
fixpoint question. It is the question of whether this compiler
evaluates the standard water tables at compile time. That is a
different sort of decision - about how much of a numerical library
belongs inside a flattener - and it is the one to put to the panel,
with this map, rather than to answer by writing code.

The chain is four links, past the three the rules allow before asking:
`h_start` <- `reference_h` <- the medium's `extends` <- a call to
`StandardWater.specificEnthalpy` <- IF97 itself.

## The Evaluate refusal was a third behaviour nobody asked for

The panel read MLS 18.3 and the answer moved twenty-four models
without a line of evaluation.

`Evaluate = true` sits in the code-generation chapter beside `Inline`
and `smoothOrder`, and its sentence is: "the model developer
**proposes** to utilize the value of the parameter for symbolic
processing. In that case, it is not possible to change the parameter
value after translation." Two things decide it. The verb is
_proposes_ - an offer from the author to the tool - and the one
consequence attached follows from _accepting_ the offer, not from
receiving it.

So there are three behaviours and the chapter names two: evaluating
takes the proposal, carrying the parameter into the run declines it,
and refusing the model is neither. The compiler was doing the third.
No tool of record answers this family with a refusal - the fluid
examples that stood here flatten and simulate elsewhere.

The measurement came before the ruling and agreed with it: a probe
that skipped the gate moved the flatten count from 793 to 817 in one
run.

### What the twenty-four did not do

Run stayed at 341. The panel predicted this precisely: under
deferral alone the family moves from "would not flatten" to
"flattened, would not run", stopping at initialisation because the
bodies behind a deferred call are not carried with the model and a
minted constant carries only a number, never an expression.

So the family is one cause wearing two carriage-shapes. The
direct-call shape (`pipe1.h_start` bound to an IF97 call) needs the
bodies walked; the named-constant shape (`heater.h_start` naming
`reference_h`) needs the mint widened to carry a binding. Both are
carrying-work of the kind the last three shifts did - not evaluation.
This compiler does not evaluate the steam tables, and the panel's
reading of where mature flatteners draw that line agrees.

### The test that had to be rewritten, and why it is not vandalism

`an_annotation_that_the_chapter_calls_an_error_is_one` held the old
refusal. Its own name is the argument for changing it: 18.8 calls
`mustBeConnected` an error in as many words, and 18.3 calls `Evaluate`
a proposal. The two `mustBeConnected` cases in that test stand
untouched; only the `Evaluate` third was moved, because by the test's
own criterion it never belonged there.

## The third attempt at complex-as-a-scalar, and the link it named

The register's instruction was to fix the key rather than the reader:
an element of an array of records should be known as a record where
flattening writes its name, not guessed at by whoever asks later. That
is the right shape, and the attempt got further than the two before
it.

Two lines were added. `components.rs` writes each element into
`acc.records` at the place `element_names` is built - the one loop
that invents `v[1]` - so the element is a record by construction.
`instantiate.rs` takes those names back up into the class's own
`records_here` before its equations are read, since that table was
gathered before any component existed.

It works, and it does not break the record's zero: the minimal case
moves from `unknown variable v[1]` to a later wall, and the whole
suite stays green - which is what tells this attempt from the two
that widened `record_class_of` and fell over `sum(arr)`.

### Why it was still reverted

The corpus does not move: 817 and 341, unchanged. The probe says why
in one line. For `BalancingStar` the elements are written correctly -
`voltageSource.v[1]` is in `acc.records` - but the equation that reads
them is not in that model. It is in `Interfaces.TwoPlug`, the base
class the source declares `P[m] = {real(v[k]*conj(i[k])) ...}` in, and
that class gathers a `records_here` of its own before its own
components are built. The take-up fills the table of the class being
instantiated; the equation is read in a table one class up the
`extends` chain.

So the missing link is named: the elements have to reach the table the
**declaring** class reads, not only the instantiating one. That is a
question about how `records_here` travels along an `extends`, and it
is the third attempt's finding rather than its failure.

Reverted whole, corpus untouched, and the next attempt starts one
question further along than this one did.

## Inline and smoothOrder: measured, and there is nothing there

The `Evaluate` ruling suggested a cheap repeat: the same chapter
holds `Inline` and `smoothOrder`, which also only propose, so if the
compiler treated either as a demand the same trade would be there
twice.

Measured rather than assumed, and the answer is a clean nothing. The
compiler reads exactly three annotations - `mustBeConnected`,
`mayOnlyConnectOnce` and `Evaluate` - and no other annotation reaches
a refusal anywhere in either crate. `Inline` and `smoothOrder` are
parsed and ignored, which is the right treatment of a proposal about
code generation.

Of the three it does read, two are the ones MLS 18.8 calls errors in
as many words, and the third is the one just corrected. So the
annotation road is now clear: nothing left that refuses a model for a
proposal the language never made.

A zero written down is worth the run that found it. Without this the
next shift would have spent the same hour on the same idea.

## The IF97 road is not a third trade: the refusal is right

The suspicion that paid twice - the compiler demanding what the
language merely proposes - was put to the IF97 family first, and this
time the answer is no. Written down because a checked "no" is worth
the run it cost.

`HeatingSystem` now stops at `cannot evaluate parameters [tank.h_start
= (reference_h + ...)]`, and the probe followed it to the end.
`reference_h` is a constant of the medium, and its binding under
`CompressibleLiquids.Common.LinearWater_pT` is

```modelica
constant ThermodynamicState state = StandardWater.setState_pT(reference_p, reference_T);
reference_h = StandardWater.specificEnthalpy(state);
```

A **constant**, not a parameter. The language gives a tool no leave to
defer a constant to the run - that is the whole of what makes it a
constant rather than a parameter - so the deferral trade that moved
`Evaluate` and `fixed` has nothing to take hold of here. The refusal
stands.

### Where the evaluation actually stops, probed

Not for want of trying, which was the surprise. A six-line model
calling `waterBaseProp_pT(101325, 298.15)` directly enters the
inliner - the body is found, eight algorithm statements and no
equations - and **thirty-eight assignments are executed** before it
gives up: `aux.phase`, `aux.region`, `aux.R_s`, `aux.p`, `aux.T` fold
to numbers, and the tail (`aux.pt`, `aux.pd`) still holds unfolded
`IF97_Utilities` calls of its own. The refusal `nothing works out
waterBaseProp_pT` names the outer call, but the outer call is not
where it stopped; it stopped on a nested one, several layers in.

So this family is neither a trade nor a road. It is the question of
how deep a compile-time evaluator follows a numerical chain, and the
measurement says this one already follows it thirty-eight statements
before running out. Whether the remaining layers are worth carrying
is a cost question, not a correctness one, and the register keeps it
apart from the walls that were simply wrong.
