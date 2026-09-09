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

## A measurement must run where the thing it measures fails

ce42b15 passed a full preflight and was pushed. The very next command
on a fresh terminal - `cargo test -p oxidelica-sim --test simulation` -
aborted with a stack overflow, and CI went red on all three platforms
and coverage. The preflight had not lied about the tree; it had
measured a different tree than the one CI runs.

The preflight builds release; `cargo test` with no `--release` builds
debug. That alone was not the fault. The fault was that the fold this
commit opened recursed deeply but not infinitely, and how deep a
recursion a run survives is set by the stack it is given. The main
thread is handed eight megabytes and finished the recursion; a test
thread is handed two and aborted partway. The release preflight ran the
body on the main thread and saw it return; the debug test ran it on a
small-stack thread and saw it die. Same code, same recursion, opposite
result, decided by which stack the run happened to be on.

This is the fourth way a build has lied about its result, after the
script that did not exist, the zero that came from nothing, and the
pipe that cut itself short. The first three were the measurement
reporting something other than what it measured; this one is the
measurement running somewhere other than where the failure lives. The
clause: a preflight runs the tests the way CI runs them - in debug, on
test threads, not only as a release binary on the main thread - because
a stack overflow hides on eight megabytes and shows on two, and the
one that ships is the two. The depth guard that closed this circle
(19c6a7c) is the real fix; this rule is so the next stack-deep fault is
caught before the push rather than by the machine after it.

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

## Stricter than the language: a genre, and a sweep for more of it

Three times in a week a family fell not to a repair but to a reading
of the specification. The pattern is worth a heading of its own,
because it is the cheapest work this project has found and it is
invisible to every other instrument.

| What was demanded             | What the language says                  | Moved              |
| ----------------------------- | --------------------------------------- | ------------------ |
| `Evaluate = true` must settle | 18.3: the developer _proposes_          | +24                |
| `fixed` must be a literal     | a Boolean attribute takes an expression | 15 to 0            |
| clock identity by arithmetic  | 16: identity is structural              | the clocked series |

The shape is always the same: the compiler treats a permission as an
obligation, and refuses a model the language allows. It is not a bug
in the ordinary sense - every one of these passed its own tests, and
each had a comment above it explaining why the strictness was
prudent. What none of them had was the sentence from the chapter.

### The sweep, and its result

The instrument was turned on the refusal texts themselves: every
`return Err` in the flattening layers, read for the ones that
_demand_ where the specification _permits_. About sixty distinct
wordings, gathered by shape.

What came back is that the remaining ones are sound. The demands for
compile-time constants - array dimensions, slicing subscripts, the
condition of a structural `if`, a clock's interval - are demands the
language makes too, in those exact places. The demands about form -
a matrix row being an array, a tuple standing only on the left of an
assignment, `cat` needing equal row counts - are grammar. The
positive-interval demand on clocks is chapter 16's own.

So the sweep found no fourth trade, and that is the finding: the
genre is real but it is not endless. Three were there, three were
taken, and the wordings that remain are the compiler agreeing with
the language rather than outrunning it.

The heading stays in the register for the next reader, with the
method attached: when a family refuses on a rule, find the sentence
in the specification before writing the repair. Twice in three tries
the sentence was the repair.

## The fourth attempt: the panel's mechanism, built whole, and the floor under it

The panel ruled that nothing needs transporting. An element's
recordness is already stored compositionally - `records["…v"]` says
what the elements are of, `sizes["…v"]` says there are elements - and
`whole_record` has been reading that pair correctly for as long as
equations between record arrays have worked. The repair was to teach
two readers to ask the same way.

Built exactly as ordered, in the four steps given:

1. `whole_record`'s stripping walk lifted into
   `record_of_a_subscripted_path`, reachable from the operator layer.
2. `record_class_of`'s `Ref` arm given the stripped second ask.
3. `apply_operator`'s operands passed through `records_written_out`
   before the promotion, so `v[1]` arrives as its fields instead of
   being wrapped in `Complex(v[1], 0)`.
4. The zero test, the suite, the reproduction, the corpus.

Every prediction the panel made about the intermediate states came
true. Step 2 alone turned the loud refusal into the quiet constructor,
exactly as foretold. The zero test stayed green throughout, because
`arr[1].x` shortens to `arr.x` and no table holds it - the stripping
form is safe where the base-name form was not, which is why attempts
1 and 2 failed and this one did not.

One correction was needed on the way: written out unconditionally, the
fields broke the scalar product built two shifts ago - the operator
was handed three values where its body wanted two. Narrowing the
writing-out to an operand that is _both_ wanted as one record and
known to be an element fixed it, and the suite went green.

### And the corpus still does not move

817 and 341, unchanged. The probe says why, and it is a floor below
everything the panel and I were looking at: at the moment
`v[k]*conj(i[k])` is expanded, the shape tables carry **no records at
all** - `tbl=0`. The asking happens inside a function body, from
`statements::one_assignment`, and every `Shapes` built there passes
`no_records()` by construction.

So the stripped ask is right and answers nothing, because the table it
asks is empty by design at that point. The question is no longer how
an element is keyed but why a body's expansion is given an empty
record table - which is a fifth thing, below the four the panel
separated.

Reverted whole. The mechanism is proven correct and harmless - suite
green, corpus unmoved, no test bent - and the next attempt starts at a
question none of the four shifts had reached: what a function body is
allowed to know about records.

## The fifth floor, named: the two tables are keyed in different forms

The fourth attempt ended on "why is a body's expansion handed an empty
record table". Probing that question further turned it into a
different and sharper one, and this is the finding of the shift.

The chain, each link measured:

1. `P[m] = {ComplexMath.real(v[k]*conj(i[k])) for k in 1:m}` is a
   **declaration's binding**, not an equation. It is spread over the
   elements by `spread_over_elements` (`components.rs:719`).
2. That function does build a proper `Shapes` with `records_here` in
   it - thirty-nine entries at the point that matters, not the empty
   table the previous probe found one layer in.
3. `collect_records` does find `v`: the probe prints
   `voltageSource.v type=SI.ComplexVoltage kind=Record`, forty-four
   times over that model, and the table holds `voltageSource.v`,
   `voltageSource.plug_p.pin.v` and the rest.
4. And the lookup still fails, because the expression being spread
   says `v`, not `voltageSource.v`. The table is keyed by **instance
   path**; the expression inside the class that declares it is written
   in **short names**. The two never meet, and no widening of the
   reader can make them.

So the empty table of the fourth attempt was a red herring one layer
too deep - the real table is full, and full of the right thing, under
the wrong spelling for the question being asked.

That also explains why the panel's mechanism was correct and inert:
the stripped ask (`v[1]` to `v`) is exactly right, and it asks a table
whose keys all carry the instance prefix. Two repairs suggest
themselves - prefix the name before asking, or key the table both ways

- and which is right depends on what else reads that table under which
  spelling. That is the next shift's first question, and it is a
  concrete one for the first time in five attempts.

Nothing shipped this shift on this family: five probes, four reverts,
one floor named. The corpus stands at 817 and 341.

### The fifth floor, corrected: the table is right and the reader never sees it

The previous entry said the two tables are keyed in different forms.
Probed one step further, that is wrong and the truth is narrower.

`spread_over_elements` holds `records_here` with eight entries, and
`voltageSource.v` is the first of them. The expression is prefixed
before expansion, so `v[k]` inside the comprehension is
`Index(Ref("voltageSource.v"), [k])` - the spelling the table uses.
Table and name agree.

Two readings were added and measured against that. `record_class_of`
given an `Index` arm answers from the base, and
`record_of_a_subscripted_path` answers a written-out `v[1]` by
shortening. Both are correct, both keep the suite green, and neither
fires: the `Index` arm answers **nothing at all** over the whole
model, and where a probe did catch a subscript the table it was
holding had `n=0`.

So the asking that fails is not the one at the spread. It is one
inside a body, where `Shapes` carries `no_records()` by construction,
and the value arriving there is already the flat name `v[1]` with no
table behind it. The spread's own asking, with the right table and
the right spelling, is never the one that decides.

Which means the repair is not in either reader. It is in what a
function body is handed - the fourth attempt's question, arrived at
again from the opposite direction and now with the alternatives
eliminated: the table at the spread is right, the readers are right,
and the body is where the record-ness is lost.

Six probes, five reverts, corpus at 817 and 341 throughout. Every
mechanism the panel described is in place and provably harmless; what
remains is a plumbing question about function bodies that no attempt
has yet touched.

### And the floor under that: the multiplication is already flat when the call is made

The plumbing question was answered by building it. A record table was
threaded through `execute`, `one_if_statement` and
`run_algorithm_sections`, and `worked_body` given a real table built
from the function's own declarations - every input, output and local
whose type is a record. It compiles, the suite stays green, and it is
the right thing on its own terms: `ComplexMath.abs` now sees its input
`c` as a `Complex`, and the probe shows `c.re` and `c.im` bound to
`voltageSource.v[1].re` and `.im` exactly as they should be.

The ten still refuse, and the last probe says why in one line. At the
call to `ComplexMath.real` the argument has already arrived as

```text
Array([Bin(Mul, Ref("voltageSource.v[1]"), Ref("voltageSource.i[1].re"))])
```

The multiplication was folded before the call was made, by the
caller, where `v[1]` is a flat name and `record_class_of` answered
`None`. Everything inside the body is downstream of a decision taken
outside it: no table given to a body can undo an operator that has
already collapsed.

So the fault is not the body's table, nor the spread's, nor either
reader in isolation - it is that `v[1]` as a **flat name** is not
known to be a record at the one moment the operator is chosen, and
the tables that hold `voltageSource.v` are keyed by the array while
the expression at that moment holds the element. The panel's stripped
lookup is the answer to exactly this, and it was measured firing
nowhere, which is the piece that still does not add up.

Seven probes, six reverts, corpus unmoved at 817 and 341 throughout,
every attempt provably harmless and none of them shipped. What the
next shift needs is not another repair but one measurement: with the
stripped lookup in place, print the operand and the table at the
moment `record_class_of` answers `None` for a name the tables should
know. The mechanism is understood from both ends; only that one
contradiction is left.

### The layout underneath: an array of records is written fields outward

The contradiction from the last shift is resolved, and it was a
measurement error on my part: the stripped lookup does fire. Built as
one binary with an environment switch, so that the two numbers cannot
be confused with two builds, it moves the corpus from **817 to 804**.
It is not inert. It costs thirteen, all of one family:
`Magnetic.QuasiStatic.FundamentalWave`, every one of them refused for
running deeper than the compiler follows.

The recursion is easy to read once seen. An element known to be a
record but never _written out_ as its fields hands the operator back
the name it was called on: `v[1]` is a `Complex`, so apply `Complex`'s
`*` to `v[1]`, whose operands are `v[1]` again, forever. Expanding the
element into its fields ends it, and then the true refusal appears:

```text
function `ComplexMath.abs` wants 2 field(s) for `c`, got 5
```

Five is the length of the array. `converter_m.iSymmetricalComponent`
is a `Complex[5]`, and `record_class_of` and `sizes` both know it -
`rec=Some("Complex") size=Some([5])`. But the flat form is written
**fields outward**: the array becomes the five `re` followed by the
five `im`, not five phasors of two fields. So `[1]` picks the reals
rather than the first phasor, and the operator was handed an array
where one record was meant.

Turning the layout around - elements outward, each element its fields

- is four lines and was measured too: **733**. Eighty-four models are
  written against the present order. That is the real floor under this
  family: not a missing lookup, not a missing table, but a layout
  decision that the corpus depends on in one direction and the complex
  operators need in the other. Reversing it is a change to how every
  array of records is named, and it needs its own shift with the
  eighty-four in hand, not a patch at the end of this one.

Eight probes, seven reverts, 817 unmoved. What is now known and was
not: the plumbing is genuinely inert (817 with it, 817 without), the
stripped lookup is _harmful alone_ (-13, one family, unbounded
recursion), the recursion has a two-line cure, and behind that cure
stands a corpus-wide layout question worth 84 models.

### The eighty-four do not have to be paid: there was no layout fault

The order to answer first - _must_ the layout change? - turned out to
be the whole of it. It must not. The claim that an array of records is
written fields outward was mine, from reading a shape rather than a
name, and it is wrong. Asked directly, the base of the subscript
answers

```text
W idx v rec=Some("Complex") size=Some([3]) baseshape=Some([3])
W idx flat=["Ref(\"v[1]\")", "Ref(\"v[2]\")", "Ref(\"v[3]\")"]
```

Three elements, outward, exactly as wanted, and `index_into` hands
back `v[2]` with an empty shape. The layout was never the fault. So
neither road (a) nor road (b) was needed as posed, and the eighty-four
were a debt against a mistake.

What is real is smaller and one file wide. `v[2]` is a single name, and
a single name is indistinguishable from a number at the point where an
operand standing where a record is wanted gets converted into one -
the conversion the library leans on so that `n*v` of a real and a
`Complex` means what it says. Taking `v[2]` for a number built
`Complex(v[2])`, a record whose real part is a whole record, and the
operator was handed a pair of pairs. That is where `[2,2]` came from,
and why `abs` reported wanting 2 fields and getting 5: not a layout
turned inside out, but a constructor applied to something already
built.

Two arms fix it, and only one of them earns its place. Reading a
subscript on an array of records as the record it is - the arm
`record_class_of` never had for `Expr::Index` - together with not
converting what is already a record: **817, unchanged, tests green,
and the twelve-line model that reproduces the whole QuasiStatic family
now flattens correctly**. Removing either turns the new test red.

The same lookup for the _flat_ written form - `voltageSource.v[1]`,
the name flattening actually produces - is now harmless where it cost
thirteen before, because the recursion those thirteen died of was the
constructor bug all along. But harmless is all it is: 817 with it and
817 without, tests identical either way. Not shipped, by the rule.

The method is the lasting part. Three shifts were spent measuring the
corpus at five minutes a turn and reasoning about shapes; the fault
fell out of a twelve-line model in one. Reproduce small, then measure.

### Where the ten stand, and what they are really waiting for

The instrument first, since the last shift shipped a fix without
saying what it moved. Two censuses from one binary, the arm in and
out: **not one model changed its refusal**. The arm fires once on the
small model and never on the corpus. The library writes
`{real(v[k]*conj(i[k])) for k in 1:m}`, the loop variable is settled
before the operator is chosen, and what arrives is a flat name. So the
fix is right and tested and corpus-inert, which is worth saying
plainly rather than leaving to be assumed.

The ten stand where they stood: `unknown variable
`voltageSource.v[1]``. But the wall behind it is now named, and named
from a forty-line model rather than from a family of machines. Bound
into `real`, the argument is

```text
handed = ["c = Bin(Mul, Ref(\"v[1]\"), Ref(\"i[1]\"))"]
```

The product was never folded into a record, so there is nothing to
bind `c.re` to, and the body's own name escapes into the flat model -
which is the `unknown variable` the corpus reports, one remove from
where it went wrong. Teaching the lookup the flat form does not help:
measured on one binary, 817 either way. The tables the operator layer
consults do hold `v`, and `record_class_of` does answer `Complex` for
`v[1]` once taught - but by then `apply_operator` has already been
entered from a shape where the nested `v[k]*i[k]` is read against an
empty table. Four candidate sites were ruled out by probe rather than
by reading: none of the four `no_records()` in `components.rs`, not
`acc.records` being extended too late, not the body table in
`worked_body`, which is genuinely correct and genuinely populated -
`recs={"c": "P.C"}` - and still leaves `c` bound to an unfolded
product.

Binding fields through the expression instead - a field of a sum is
the sum of the fields - handles `+` and `-` and not `*`, which is
exactly the case in hand: the real part of a product is not the
product of the real parts. So the fold has to happen in the operator
layer, before the call, and the remaining question is which caller
reads the multiplication against a table that does not hold `v`.

Three of the C-fluid guesses died the same cheap way. A constant whose
value is a call flattens; through a replaceable package it flattens;
through a chain of extending packages it flattens. Whatever the
twenty-five die of, it is none of those, and an hour of corpus runs
was saved by four minutes of small models.

The queue, kinds folded together: **cannot evaluate parameters 64,
has no value 64, structurally singular 69, unknown variable 52, two
equations for der 17, unknown function 10**. The three families closed
this week have left the top; what stands there now is parameters that
want a value the compiler will not compute.

### The fluid twenty-five: a constant array of records, read by element

The C-fluid family answered to the small-model rule at once, and not
the way three shifts of guessing expected. A constant whose value is a
call flattens. Through a replaceable package it flattens. Through a
chain of extending packages it flattens. All three are in
`tests/small`, all three green, and each was a candidate the corpus
would have taken five minutes to rule out.

What actually refuses is `ModelicaTest.Fluid`'s

```text
unknown variable `SourceP1.medium.fluidConstants[1].molarMass`
```

and in twenty-six lines it is this: a medium declares

```modelica
constant FluidConstants fluidConstants[nS] = {FluidConstants(
  molarMass = 0.018, criticalTemperature = 647.1)};
```

and a model reads one field of one element. The refusal that comes
back names the fault outright:

```text
an equation between shapes [] and [2]:
  Ref("medium.MM") = Member(Number(0.018), "molarMass")
```

`Member` of a _number_. The array of one record was substituted as its
first field, so `[1]` picked `molarMass`'s value and `.molarMass` was
then read off that. A record is its fields and an array is its
elements, and by the time the subscript is read both are the same kind
of value - the same ambiguity as the QuasiStatic family, one layer
further in, and this time in constant substitution rather than in the
operator layer.

Two fixes at the subscript were built and measured against the small
model: neither moves it, because the collapse happens before either
runs - the constant arrives already folded to `Number(0.018)`. So the
work is in `substitute_class_constants`, where an array of records
must be substituted as an array of records rather than flattened into
the fields of the first. Named, reproducible in a second, and left for
the next shift with its price in hand rather than as a question.

### Two walls down, and the first look at the largest kind

The comprehension was the wall the quasi-static families stood at, and
it took one line. Each turn of `{real(v[k]*conj(i[k])) for k in 1:m}`
builds its own view of the names in scope, because the loop variable
now has a value - and that view was built with an empty record table.
So the product of two phasors was arithmetic on two names, `real` was
handed something that is not a record, its input had nothing to bind
to, and the body's own `c.re` escaped into the flat model. The kind
falls **5 to 1**, and the five that move are the ones this started
from three shifts ago: they now flatten their power equations and
stop at _unbalanced model_, a later and different fault.

Totals hold at 817 and 341, which is the lesson to keep next to the
number: moving a model from one refusal to a later one does not
flatten it. Only the census shows that work happened. Flattening cost
1008ms then 1021ms per model against 986ms before the shift - inside
the run-to-run spread, watched because it was asked for, and not a
price worth naming.

The constant-array fix is the same ambiguity one layer in: a record is
its fields, an array is its elements, and once worked out they are the
same kind of value. Written out and subscripted, `[1]` took the first
_field_. Three arms fix it, each using the written form to tell the
two apart, and the small model answers 0.018 for one field and 647.1
for the other - by name, not by luck. The corpus does not move,
because the library reaches those constants by _name_, and that path
asks a table that is empty at the moment of asking. Same shape of
fault as the comprehension, different site, still open.

**Structurally singular, 69** - the largest kind, never touched. Three
probes say it is not one family. It splits by what cannot be matched,
the biggest group being five table models, and the refusal names its
own cause outright:

```text
equation Ref("d_t_new.u") = Ref("t_new.y[1]") cannot be matched
  (differentiation recursed through a cyclic definition)
```

That is `Modelica.Blocks.Continuous.Der` differentiating a table's
output, and the tests chain two of them. So the kind is at least three
things: differentiation that runs away, multibody models where a
rotation cannot be matched, and kinematics. Three attempts to
reproduce the first in a small model all flatten - a `Der` on a table
output flattens, two chained flatten, a stepwise `if` flattens - so
what makes the real one recurse is not yet in hand. The next shift
starts there, with the probe already pointed at `differentiate_at` and
its depth limit.

### The largest kind, halved: it was differentiation, not matching

Two rules, two probes, and _structurally singular_ falls **69 to 46**.
Neither fault was in matching or in index reduction, where the kind's
name points. Both were in differentiation, and both were found by
squeezing a failing model rather than by writing one.

That method is the finding worth keeping. Three invented models -
a `Der` on a table output, two chained, a stepwise `if` - all
flattened, because the cause was in none of them. Squeezing
`CombiTimeTable.Test55` by dropping components while the refusal
survived reached eleven lines, then nine, and the difference was
visible at once: a table _written out_ in the model is short, and one
_read from a file_ is a hundred rows deep. The refusal even said so,
if read closely enough - it blamed a cyclic definition, and nothing in
differentiation follows a name to its equation, so no cycle can arise
there. The limit of thirty-two was guarding the stack under a
cycle-shaped message. Raised to four thousand, the message corrected,
five table models run.

The second was plainer once the first was out of the way. The library
writes `Modelica.Math.sin`; what reaches differentiation is `.sin`,
the function's own name with its package path resolved to nothing in
front of it. Rules were matched against the whole string, so a rule
this compiler has had all along was not found, and every model holding
a sine source came out structurally singular. Squeezed to nine lines -
a sine current, an inductor, a ground.

**Running 341 to 358, runnable 338 to 355**, floors moved with each
commit. Flattening 1005 then 1015ms per model against 1021 - unchanged,
because neither fix touches the flattening half at all.

What is left of the kind, counted rather than guessed:

- **22** cannot differentiate _through an algebraic variable_ - the
  multibody group, a different mechanism.
- **12** other, no reason given in the message.
- **11** `abs`, which is genuinely not differentiable at zero, and the
  question is whether these models want `sign(x)*der(x)` or whether
  the refusal is right.

The next shift starts at the twenty-two, and the first step is to
squeeze one of them.

### `abs` under a derivative: what is actually being decided

Eleven models are refused for `cannot differentiate function abs`,
and the shape of the question is not what the name suggests.

Where it comes from, in every one of them, is
`Modelica.Blocks.Sources.KinematicPTP`:

```modelica
aux1[i] = p_deltaq[i]/p_qd_max[i];
sd_max = 1/max(abs(aux1));
```

`aux1` is a quotient of two parameters. Its derivative with respect to
time is zero, and so is `abs(aux1)`'s, discontinuity or not - the
argument never moves. The refusal is not protecting anyone here; it
fires because differentiation asks the question structurally, without
noticing that the answer is zero either way.

So the fork is:

1. **A rule for `abs`**: `der(abs(x)) = sign(x)*der(x)`. What every
   other tool does, and what the language means. It is wrong only at
   exactly zero, where `sign` is zero and the true derivative does not
   exist - a measure-zero disagreement that no solver visits by
   chance. Cheapest, and takes eleven models.
2. **Zero where the argument is constant**: notice that `der(u)` is
   zero and answer zero without needing a rule for the outer function
   at all. Narrower, correct everywhere, and it generalises to every
   function of parameters rather than to `abs` alone. Slightly more
   work, and it would not help a model that genuinely differentiates
   `abs` of a moving variable.
3. **Leave it**: the refusal is honest about a function that is not
   differentiable, and eleven models wait for a decision.

The second is the one this compiler's habits point at: it refuses
rather than guesses, and it answers what it can actually work out.
But it is a choice about what the tool promises, not a repair, and it
is left here for Roman rather than made in a shift.

### Two limits that guarded against nothing, and one rule that is true everywhere

The `abs` fork was decided the second way: nothing about `abs`, but
_whatever does not move has a derivative of zero_. It is the same
move as `Evaluate` and `fixed` before it - not a mechanism added, but
a requirement dropped. `KinematicPTP` writes `1/max(abs(aux1))` where
`aux1` is a quotient of two parameters; differentiation was asking
structurally and never noticing that the answer is zero whichever
function stands around it.

The strictness is where the work is. "Does not move" means every leaf
is a literal, a parameter, or an algebraic name whose own definition
does not move - followed one step at a time and never through itself,
so a definition mentioning its own name cannot be read as constant.
Not `time`, not a state, not a dummy derivative. `abs` of a state is
still refused, and a test says so, because that is where the
discontinuity is real and an answer would be a guess. **Running 358 to 363.**

Then the reduction limit, and it is the third time this month the same
shape has turned up: a limit written to guard against a loop, guarding
instead against depth that is perfectly legitimate. The instrument
settles it in one run - count what the matching covers at each step:

```text
R step=0  matched=173/248
R step=8  matched=233/258
R step=15 matched=252/266
```

Gaining ground every time. A system making progress is not in a loop,
and a model of genuinely high index needs as many reductions as it has
index. Raised from sixteen to two hundred and fifty-six, the kind
`still cannot be matched` goes to **zero** and those six reach what is
actually wrong with them - one of them a diverging algebraic loop.
Totals hold, because reaching a later wall is not running.

What the singular kind is now, **39** from 69 at the start of the
week: **20** cannot differentiate through an algebraic variable, **7**
`abs` of something that genuinely moves, **12** others.

The connected-current case is not taken. The shape is understood -
`r.p.i` is pinned only by connection equations that mention each
other, and `p.i` is already in the table one equality away - but
grounding a plain name-to-name equality early was built, measured
against the small model, and does not move it: solved out of a
connection equation, `-p.i + r.p.i = 0` arrives as `-(-r.n.i)/-1`, and
reading through the wrappers still leaves the chain unclosed. Reverted
rather than shipped. The small model stays red, which is the honest
place to leave it.

### The seven remaining `abs`: both measurements say the refusal is right

Two facts were asked for before reopening the fork, and both came back
against the `sign` rule.

**Does the argument cross zero?** In these seven, yes, and on purpose.
They are `ModelicaTest.Fluid.TestUtilities.TestRegRoot2Derivatives`
and its neighbours - models written to test what happens _at_ zero -
and the driving equation is

```modelica
x = time - 1;
```

a ramp through zero at the middle of the run. The names say it too:
`TestRegRoot2ZeroDerivative`, `TestPressureLossDerivatives`. Zero is
not a corner these models avoid, it is the thing they are about.

**Is `abs` expanded through `noEvent`?** No. It is computed directly -
`"abs" => Some(Unary::Abs)` in the code generator, `x.abs()` at the
run - so no zero crossing is generated and the solver steps straight
through the discontinuity. Which means a `sign(x)*der(x)` rule would
be wrong exactly where these models look, on a step the solver takes
without stopping, and nothing would say so.

So the fork closes the other way from where it looked. For the eleven
it was never about `abs` at all, and the general rule took them. For
these seven the refusal is correct and the honest answer is that
differentiating `abs` across zero needs the event that the language
asks for and this compiler does not yet make. That is a real piece of
work - expand `abs`, `sign`, `max`, `min` into their conditional forms
so the solver stops at the crossing - and it is worth doing for the
run half generally, not as a way to take seven models.

### What is left of the singular kind, and a warning about `when`

The wrapper folding took the kind from 39 to 37 and moved three models
on. The remainder splits into two shapes, and the first is now clear.

The connected-current case is a **cycle, not an ordering**. With the
wrappers folded the candidates read plainly, and what they say is

```text
pmActuator.r.p.i := pmActuator.r.i
pmActuator.r.i   := pmActuator.r.p.i
```

each waiting for the other, with `r.n.i` doing the same in the other
direction. No amount of folding or reordering closes that: what pins
these currents is the connection set as a whole, one equation short of
determining any of them individually. Reading it needs the set, not
the definitions - which is a different piece of work from anything
tried so far, and the small model stays red pointing at it.

The second shape looked like the `abs` rule again and is not.
`Trapezoid` writes

```modelica
when time >= (pre(count) + 1)*period + startTime then
  count = pre(count) + 1;
  T_start = time;
end when;
```

and reads `T_start` in its output equation. Between events `T_start`
holds, so its derivative there is zero - the same reasoning that took
the eleven. Built that way, threading what `when` clauses assign into
differentiation as definitions worth zero, the small case does pass:
the refusal moves past `T_start` to the next thing.

**But the corpus goes 37 to 39 and the sub-kind 20 to 24.** Measured,
reverted, not shipped. The rule is true between events and the trouble
is what it does at them: a variable that jumps is not one whose
derivative is zero everywhere, and index reduction that has demoted a
state on the strength of a zero derivative has built something that
stops being true the moment the `when` fires. So this is not a fix to
tighten but an idea to be careful with - and the census caught it,
which is the argument for reading the census and not only the totals.

### The seven `abs`, counted properly, and where the multibody prize is

The suspicion about `zeroDerivative` was right about the mechanism and
wrong about these seven. Reading the option is worth **two models**
and is shipped; but `regRoot2` and its neighbours carry
`annotation(smoothOrder = 2)`, not a derivative rule, so there is
nothing to use and the body has to be differentiated after all. The
`abs` in it is real, its argument crosses zero on purpose, and the
refusal stands - which is what the two measurements last shift already
said.

Expanding `abs` and `sign` is written off here so nobody returns to
it. The specification expands them **under `noEvent`**, so the
expansion generates no crossing, and differentiating the expanded form
branch by branch gives back exactly `sign(x)*der(x)` - the same wrong
answer at zero, reached the long way round. There is no win in that
direction. What these models want is an event at the crossing, which
`noEvent` explicitly forbids: they want the library's own derivative,
and the library gave one only for `regRoot`, not for `regRoot2`.

The connected current was counted before building, and the count says
don't. Three models sit behind it - `pmActuator.r.p.i`,
`actuator.r.p.i`, `saturatingInductor.p.i` - against work that means
reading a connection set as one system rather than as definitions.

The count also found where the prize actually is. Of the twenty
`cannot differentiate through an algebraic variable`, the orientation
of a multibody frame accounts for **twelve**: six on `R.T` and six on
`R.w`, with three more on `r_0` and one on `delta_0`. That is one
family four times the size of the connection-set one, and it is where
the next shift should squeeze.

### The multibody fifteen: the definition is thrown away by the skip

Squeezed from `Pendulum` to nine lines - a world, a revolute joint, a
body - and the refusal survives, naming `rev.frame_a.R.w[3]`. The
probe then says the whole of it in two lines:

```text
M3 rev.frame_a.R.w[3]    def=None
M3 world.frame_b.R.w[3]  def=Some("Number(0.0)")
```

The world's own angular velocity is zero and known. The connection
puts `world.frame_b.R.w[3] = rev.frame_a.R.w[3]` in the system, which
would carry that zero across. But definitions are gathered while
skipping the equation under reduction, and at that step the equation
under reduction _is_ the connection - so the joint's side is left with
nothing, and a frame that never turns is called something nothing can
differentiate. Fifteen models die of it.

The skip is right in general: an equation cannot define its own way
out, or `u = 3` would be read instead of `u = 2*x`. Narrowing it to
the one name being reduced for, so the connection still says what it
says about the _other_ side, was built and measured: it takes the
small model and the corpus goes **363 to 351**, with two tests
failing. Reverted.

So the definition is correct and the trouble is downstream of having
it - the same shape as the `when` rule last shift: true in itself,
harmful to what the reduction then does with it. Somewhere a step is
relying on that name having no definition, and finding which one is
the next move rather than another attempt at the gathering. The small
model is kept red and says so.

### `noDerivative` is exactly the right idea and does not fit yet

The theory was right about the library. `Frames.resolve1` carries

```modelica
annotation(derivative(noDerivative = R) = Internal.resolve1_der,
           InlineAfterIndexReduction = true);
```

and its rule takes `(R, v2, v2_der)` - an orientation handed over whole
with no rate of its own. That is precisely how the multibody library
avoids ever forming `der(T)`: the rule is written for whatever `R` is,
so nothing has to differentiate a rotation matrix.

`noDerivative` and `zeroDerivative` say the same thing about the
_shape_ of the rule - the named input gets no derivative beside it -
so reading it is the same two lines. Measured: **flatten 819 to 717**.
The rules are found and applied, and then the values do not fit: an
orientation is a record of a three-by-three and a three, the rule is
called with it whole, and what comes back is refused for being an
array where a scalar was wanted. So the annotation is not the missing
piece by itself; the record-valued argument has to travel into the
rule the way the record's own operators now travel, which is the work
`InlineAfterIndexReduction` is hinting at on the same annotation.

Reverted, measured, written down. Two shifts running, the mechanism
named by the maintainer was real and the fix was one layer further in
than it looked - which is the argument for squeezing first and
theorising second, and for measuring before shipping either.

### The unwritten law: a definition changes which states survive

Three shifts running, a rule true in itself cost models and no account
of why. `sign(x)*der(x)` for `abs`. "What a `when` assigns does not
move." Narrowing the skip so a connection still defines its other
side. Each was correct where it was checked; each lost ground when
measured.

The instrument settles it in one run. Print the state demoted at each
reduction, run the corpus with the change behind a switch, and diff
the lists:

```text
Pendulum        off: 6 victims   on: 1
DoublePendulum  off: 13 victims  on: 10
```

The states are not the same states. Definitions are how index
reduction decides what it can reach _through_, so supplying one takes
a variable out of the running as a state and the whole selection moves
behind it. The 363 models that run stand on the present choice, and
they were not chosen for it - it is simply what the compiler has been
doing.

Measured properly, the third attempt is smaller than it looked: from
one binary, **363 with the change off and 354 with it on** - nine
models, not twelve, and none of them stop _flattening_. The census by
`built` shows nothing at all, because the loss is entirely in the run
half. That is worth knowing on its own: a change can be invisible to
the refusal census and still cost nine models.

So the law, and it goes in AGENTS.md: **a change that gives a name a
definition changes which states reduction keeps, and the measurement
is the victim list, not the model count**. A definition-adding change
is not wrong for costing models and not right for taking a small one.
If the victims move, what has been built is a different compiler
rather than a fix - and it needs to be argued as one, with a reason
why the new selection is better, not merely different.

This explains all three reverts after the fact and should prevent the
fourth.

### Where `noDerivative` actually stops, probed rather than guessed

Behind a switch, the refusal names its own place:

```text
an array value cannot be used where a scalar is expected:
  Array([Bin(Add, ..., Bin(Mul, Ref("R.T[1,1]"), Ref("$seed1[1]")), ...
```

and the probe on what is handed to the rule says the rest:

```text
handed = ["Ref(\"R\")", "Array([Ref(\"$seed1[1]\")...])", "Ref(\"$seed1\")"]
shapes = [[], [3], [3]]
```

The seed is declared with the shape `[3]` and handed over as the
single name `$seed1`. The rule's body reads `v2_der[1]`, which is not
something one name can be read off - so the fault is not the record
argument at all, as it looked from the corpus number. It is the
_seed_ for an array argument.

Handing an array of element names instead was built and does not
finish the job: the same refusal comes back one layer in, now on the
orientation itself. Both parts are needed - a seed that is an array,
and a record travelling into the rule whole - and the second is the
larger one.

`InlineAfterIndexReduction = true` sits on the same annotation and is
the hint worth following first. It says: keep this function opaque
until reduction is done, use the rule for the derivative, expand the
body afterwards. If the order is what matters, then a record has to
travel only into the rule, not through the whole inliner - which is
the smaller of the two jobs and the one to try next.

Reverted, and the two measurements from before still stand: 819 to 717
in the round, nine models in the run half for the narrowing. What is
new is that neither number is the obstacle any more, because the
obstacle now has an address.

### `InlineAfterIndexReduction`: honoured, and the bill it presents

The annotation is real, it is on the frame transforms beside their
`derivative(noDerivative = R)`, and this compiler was not reading it
at all - so those calls were inlined before reduction ever saw them.
What reduction met was a body full of matrix entries, and the
orientation of every frame became something it had to differentiate.
That is the whole multibody family, and the diagnosis is confirmed:
honouring the annotation changes what the refusal says, from _cannot
differentiate `R.w`_ to a shape complaint about `R` itself. The call
now stands, and the record travels.

The bill is **819 to 781** - thirty-eight models - and every one of
them dies the same death:

```text
an array value cannot be used where a scalar is expected:
  Array([Array([Ref("body.frame_a.R.T[1,1]"), ...
```

**43 refusals of that one shape** across the corpus. So the work is
not spread out, it is a single wall: a call left standing is handed a
record, and the machinery that reads a standing call's arguments
wants a scalar. Which is the third appearance of one fault - a name
standing for an aggregate while the reader counts it as a scalar,
after `v[1]` as a record and the constant array of records - and both
of those were fixed by teaching the reader the shape from a table
rather than rebuilding anything.

So the order for the next shift is clear and the work is bounded: the
argument of a standing call has to be allowed to be a record, in one
place, `names.rs` where a scalar is demanded. Then the annotation
costs nothing and the fifteen multibody models get their rule instead
of a matrix to differentiate. Measured against the law: the victim
lists have to be diffed before this ships, because a call that stays
whole is a definition that reduction can no longer see through.

### The aggregate-as-scalar fault, third time, and a walk of the rest

A name stands for an aggregate and the reader counts it a scalar. That
is now three, in three different places: `v[1]` of a `Complex[3]` at an
operator, an array of record constants read by element, and an
orientation handed to a call that stays whole. Each time the fix was
to tell the reader what the shape is, never to rebuild anything.

The third is narrowed by declaration rather than by loosening the
check: the input the argument stands for says whether an aggregate was
asked for. A record or a dimension allows one; a scalar there refuses
exactly as before.

Its worth is only visible with `InlineAfterIndexReduction` honoured,
which is what makes calls stand. Behind the switch, together: **781
back to 819**, the whole wall gone, and by the law's own instrument -
the diff of which models run - the cost is **one model, named**:
`Rotational.Examples.GenerationOfFMUs`, whose adaptor carries
`derivative(noDerivative = q_qd)`, a rule this compiler still reads
past. So the call stands with no rule to differentiate through, which
is exactly the gap `noDerivative` fills. Not shipped together for that
reason.

The preventive walk, since the fault has come three times. Every place
that demands a scalar, and whether the demand is honest:

- **arrays.rs, 14** - loop bounds, subscripts, conditions, elements of
  a comprehension. All honest: each is a place the language itself
  says is a scalar.
- **builtins.rs, 6** - arguments of `sin`, `div`, interpolation
  points. Honest: these are declared scalar by the specification.
- **equations.rs, 3** - `when` conditions, `if` conditions, the value
  of an assignment inside a clause. Honest.
- **extents.rs, 2** - an assertion's condition and a range item.
  Honest.
- **names.rs, 1** - the call argument. This was the dishonest one, and
  it is now narrowed by declaration.

So the walk finds no fourth waiting, which is worth as much as a fix:
the remaining demands are the language's own, not a blind reader. What
would bring a fourth is a new road that carries records somewhere they
have not been - and the guard against that is this list rather than
another discovery.

### The one victim, judged rather than counted

The law asked for a reason, not a score, and the reason turns out to
be ours. `Rotational.Examples.GenerationOfFMUs` is the single model
lost when `InlineAfterIndexReduction` is honoured, and the victim
probe says what happens to it:

```text
off: 5 states demoted    on: 2, then it stops
     directInertia.inertia.phi
     springDamper.springDamper.phi_rel   <- no longer demoted
```

The first suspicion was the adaptor's own
`derivative(noDerivative = q_qd)`, which this compiler reads past.
Reading it was built and measured, and it does not save the model -
worth knowing on its own, and worth knowing that reading it _alone_
costs 819 to 779, while with late inlining it is 819 again. The two
are a pair: a rule that is only reachable once the call stays whole,
and a call that only stands where its rule can be read.

What actually stops the reduction is plainer, and it is not a
legitimate opacity at all:

```text
cannot differentiate through `torqueToAngle2b.w_internal`,
  differentiating (w_internal - inertia2b.w) = 0
```

and `w_internal` is defined by an ordinary equation of the adaptor -
`w_internal = der(phi)` - with no annotation anywhere near it. So
reduction _could_ reach through: `phi` is a state, its derivative is
named, and demoting `phi` is exactly what the off case does. It stops
because the definition-gathering does not look through a `der(...)`
on the right-hand side to find the state under it.

By the maintainer's own test that makes this **work, not a price**.
Nothing ships that drops the run floor: the narrowed scalar check is
in, on its own and inert, and both halves of the annotation work stay
behind the switch until the reduction can follow `w_internal = der(phi)`
to `phi`. That is one named gap rather than a judgement call, and it
is where the next shift starts.

### The series, measured whole, and the one gap that still holds it

All four parts together - reading `noDerivative`, honouring
`InlineAfterIndexReduction`, allowing an aggregate where the input was
declared one, and gathering a definition through `der(...)` - come to
**819 flatten and 362 run**. The flattening wall is gone entirely: the
38 that the standing calls cost are back, and nothing else moves. The
run list, which is the instrument the law asks for, loses exactly one
model and gains none.

It is the same one as before, and the last part was aimed at it. The
aim was right and the shot missed by a layer. `w_internal = der(phi)`
does reach the gathering, but not in that form: by then the model
reads

```text
torqueToAngle2b.w_internal = torqueToAngle2b.w
```

because `der(phi)` was already given a name of its own upstream, and
`w` is that name once more removed. So `as_der_of` finds nothing to
look through - the derivative is not written as a derivative any more,
it is an ordinary unknown three equalities away from the state. The
gathering would have to follow that chain rather than one step.

So the series stays out of the tree, and the floors stay where they
are. What it is worth is now exact rather than estimated: **the whole
multibody family's wall for one model**, and that one model is a chain
of aliases away from working too. That is a good trade to make once
the chain is followed, and a bad one to make on a promise.

The instrument built this shift is what made the difference: every
question above was answered by `library check --only <Class>` in under
a second, where the same questions cost four corpus runs yesterday.

### The trade, named correctly, and where the sixteen stand now

The trade was stated wrong last shift. With the series in: flatten
819 to 819, run 363 to 362. The thirty-eight that standing calls cost
come back, and **the multibody family does not arrive** - so what the
series buys today is correctness and no gain, at minus one model. Not
"a family against one".

The new instrument answers the rest in a minute, one second per model.
All sixteen were run under the series, and the answer is outcome (a),
without exception: every one of them now stops at the same wall, and
it is a wall the earlier one was hiding.

```text
PrismaticConstraint      unknown variable `R2`
DoublePendulum           unknown variable `R_rel`
HeatLosses               unknown variable `spring1.lineForce.frame_a.R`
RollingWheel             unknown variable `R`
Engine1a                 unknown variable `R_rel`
...  sixteen of sixteen, all of one shape
```

An orientation is stored as its fields - `rev.R_rel.T[1,1]` and the
rest - and a call that now stays whole asks for `rev.R_rel`, the name
of the record itself, which nothing in the compiled model holds. So it
is the aggregate-as-scalar fault a fourth time, and this time in the
run half rather than in flattening: `code.rs` looks a name up in its
slot table, finds no slot for a record, and refuses.

That is a good place for it to be. The wall is single, named, and in
one file, and the fix has the same shape as the three before it - the
reader has to know that a record name stands for its fields rather
than for a slot. Which means the series ships together with that, in
one go, exactly as the maintainer proposed for outcome (a).

`GenerationOfFMUs` is then not the series' last obstacle but its only
price, and the question about it is separate: a derivative loses its
origin when it is renamed, and following the aliases afterwards cannot
recover what the renaming dropped. That wants the `der` trail carried
in the table rather than reconstructed by search.

### Where the fourth appearance has to be fixed, and where it must not

The wall the sixteen now stand at is in the run half, and the obvious
place is the wrong one. `code.rs` refuses because its slot table has
no entry for `rev.R_rel` - but a record has no slot by design, it has
one per field, and giving it one would be inventing a value the model
does not hold.

Where the arguments of a standing call are taken apart is a few lines
further on, and it already does the right thing for arrays:

```rust
Expr::Array(items) => items.iter().for_each(|item| leaves(item, out)),
one => out.push(one),
```

An array written out is walked to its leaves; a record named by one
name is pushed whole, and then looked up, and then refused. The fix is
to let a record name be walked the same way - which needs the fields
in the order the record declares them, and that is knowledge the
flattening half has and the run half does not.

So the fix belongs upstream, where the call is still being built and
the record table is still in hand: a standing call's record argument
is written out as its fields there, and the run half then sees exactly
what it already knows how to walk. That is the same answer as the
three earlier appearances - tell the reader the shape, at the place
where the shape is known - rather than teaching a new reader to guess.

With that, the series is one commit: `noDerivative`, the annotation,
the aggregate argument, and the record written out at a standing call.
The instrument makes the check cheap - sixteen models, one second
each - and the floors move only if all sixteen arrive.

### The record at a standing call, and the prefix that is missing from it

Writing the record out where the call is built is the right place and
does not finish the job, and the reason is worth having exactly.

The argument arrives at the standing call as `Ref("R_rel")` - no path
in front of it. The flat model holds `rev.R_rel.T[1,1]`. So three
lookups all miss for the same reason:

```text
W4 R_rel  in_table=None  class=Frames.resolve2  decl=None
```

The table of records is keyed by the model's own paths, and `R_rel` is
not one of them. The callee's declarations do not have it either,
because `R_rel` is not a component of `resolve2` - the input there is
called `R`, and `R_rel` is the _caller's_ name for what it passed. So
the name at the call site belongs to neither party's table: it is the
argument as the joint wrote it, with the joint's prefix already
stripped somewhere between the joint and here.

That is the missing link, and it is a different one from what was
expected. Not "teach the reader that a record is its fields" - that
part is written and correct - but "the argument of a standing call has
lost the prefix that says whose record it is". A name without its path
cannot be looked up in any table, whatever the table knows.

So the ladder stopped at its first rung, which is what it is for: the
small model still refuses, and none of the sixteen were run, and
nothing was shipped. The next move is to find where the prefix comes
off - the argument is prefixed for every other purpose, since
`rev.R_rel.T[1,1]` exists in the flat model - rather than to teach
another reader to guess at unprefixed names.

### The asymmetry, found in one run: it is a body's own input, not a lost prefix

The instruction was to compare the two roads rather than hunt a loss
along the pipeline, and one probe settled it. The same call, both
arguments side by side:

```text
CMP-LATE resolve2 args=["Ref(\"R_rel\")", "Array([Ref(\"rev.frame_b.R.w[1]\")...])"]
```

The second argument is flat and prefixed. The first is bare. And
everywhere else in the same model the name _is_ prefixed and written
out:

```text
rev.frame_a.R.T[1,1] = rev.R_rel.T[1,1]*rev.frame_b.R.T[1,1] + ...
```

So nothing strips a prefix. `R_rel` at that call is not the joint's
component at all - it is the _input of `Frames.absoluteRotation`_,
whose body reads

```modelica
R2 := Orientation(T = R_rel.T*R1.T, w = resolve2(R_rel, R1.w) + R_rel.w);
```

Inlining that body binds `R_rel.T` and `R_rel.w` field by field, which
is why every other use came out right, and leaves `R_rel` itself
unbound - because a record has no single value to bind it to. The
inner call keeps the body's own name, which belongs to no model and no
table.

That is the invariant stated plainly and broken: anything surviving
into the run must carry the flat model's names, and this carries a
function-local one. The fix follows from the fault rather than from
the symptom - when a record input is bound field by field, a call
inside the body that takes the record whole has to be given the
caller's argument, written out, in place of the body's name for it.
Not a reader taught to guess at bare names, which would be the fifth
appearance of the same mistake in a new place.

### Restoring the name works; the sixteen still do not arrive

Road one was the right one and it is two lines. The check asked for
came back exactly as predicted:

```text
B2 R <- R_rel            (fields bound: R.T, R.w)
B4 body-bind R <- Array([Array([Array([Ref("rev.R_rel.T[1,1]") ...
```

Two roads bind a record input, and both leave the input's own name
unbound - one has the caller's name in hand, the other has the fields
already written out. Binding the name on each is small and exact: the
body's `R_rel` becomes what the flat model calls it. `unknown variable
R_rel` is gone from the small model, and the refusal moves on.

But the ladder's second rung says stop. All sixteen still flatten and
none run, and the wall that was single has split in two:

- most now reach differentiation and stop there - `cannot
differentiate through an algebraic variable`, `cannot differentiate
this expression`;
- some still stop on a record's name, but a different one:
  `fixedFrame.frame_a.R`, `spring1.lineForce.frame_a.R` - records
  reached through a _connector_ rather than passed as an argument.

So the fix is right and incomplete twice over. The connector case is
the same fault at a place the argument binding never sees, and the
differentiation case is the wall behind it, which was always going to
be there and is now visible.

Nothing shipped: the sixteen were the condition, and they did not
arrive. The floors stay at 819 and 363, and the two remaining shapes
are named rather than guessed at - which is what the rung is for.

### The chain rule applied at last: four walls, and the fourth is a different animal

One model, every wall knocked down locally and crudely, nothing kept.
The question was only whether the chain has a bottom. It has, and the
bottom is not what the line was aimed at.

The walls, in the order they appeared:

1. **`Index(Call(resolve2, ...))`** - a subscript on a standing call.
   The message said only "cannot differentiate this expression"; made
   to name itself, it named this. Knocked down by differentiating the
   subscript through its base.
2. **`Array([...])` under a derivative** - the written-out orientation
   itself. Knocked down element by element.
3. **`cannot differentiate through R.w[2]`** - an algebraic with no
   definition to reach. Knocked down by pretending it has a derivative
   of its own name.
4. **`equation w[2] = rev...w[2] constrains no state`** - and this one
   does not fall, because there is nothing behind it to differentiate.

The fourth is the answer. `world.frame_b.R.w[2]` has two equations:

```text
world.frame_b.R.w[2] = 0
world.frame_b.R.w[2] = rev.frame_a.R.w[2]
```

The world's frame does not turn, so its angular velocity is zero by
declaration, and the connection then says the joint's inboard frame
has the same. Neither equation mentions a state. Index reduction has
nothing to do here at all: this is not a high-index system needing
differentiation, it is a **redundant pair the matching cannot use** -
one unknown, two equations, and a matching that wants each equation
matched to an unknown of its own.

So the line's diagnosis was right about every wall it named and wrong
about where it ended. The multibody family does not need a
derivative rule for rotations, nor a record travelling into a rule.
It needs the matching to handle a connection between two things that
are both already determined - which is a different subject from index
reduction, and the fifth appearance of the record fault
(`fixedFrame.frame_a.R`, through a connector) sits on the same road.

Honest count of the line: five shifts, four correct changes, cost one
model, gain zero. Every one of the four is right by the specification
and needed. But the bottom of the chain is somewhere else, and that is
a decision to take rather than another shift to spend.

### Parking the multibody series, with numbers rather than a shrug

The panel retracted three readings, one of them the maintainer's, and
the line is parked. What is recorded here is what was measured, so the
next approach starts from fact.

**Edit (4) is not free in isolation.** Re-derived from the session log
(its source was in no commit) and run alone against the corpus: **818
flatten, not 819**. It costs exactly one model, named by diffing the
flatten lists:

```text
ModelicaTest.Media.TestAllProperties.IncompleteMedia.ReferenceAir_dT
  an array value cannot be used where a scalar is expected: Array([Number...
```

Binding a record's own name to the written-out array lets a body's
inner call take the record whole - which is the point - but a media
function elsewhere reads that same name where a scalar is wanted and
now refuses. So edit (4) is safe only with its companion, the
standing-call reader that turns such a name back into fields. That is
the concrete reason the series is one commit and not four: the parts
are not independently floor-safe. The patch is saved at
`docs/edit4_record_name_array_branch.patch.txt` so it need never be
reconstructed from a log again.

**`GenerationOfFMUs` was lost to a defect, not to a price.** The early
return in `inlining.rs` for `InlineAfterIndexReduction` hands back a
bare `Expr::Call` before the `noDerivative` machinery can wrap it in a
`WithDerivative`. So the rule the series reads is never attached to a
standing call at all, and that model's one lost run is this defect
rather than the annotation's cost. The next approach must not pay for
it twice.

**The refusal text is not a family's signature.** It quotes the
highest-indexed member of the deficient subset, and connection
equations are appended last, so reversing equation order changes which
equation is named. Reading a refusal as a diagnosis is not a
measurement - the panel proved this by moving the named equation
without changing the model.

**`to_unit1` is a general import fault, reproduced in 26 lines.** A
function imported into a class by a deep single-name path -
`import Modelica.Units.Conversions.to_unit1` in `Parts/Body.mo` - is
lost when it appears inside a _component modification_
(`Shape sh(lengthDirection = to_unit1(r_CM))`): the modifier is
written in `Body`'s terms but worked out in `Shape`'s scope, where the
import does not reach. It surfaces at the run as `unknown function
`to_unit1``. The address is `components.rs` where a modifier is
expanded with the child's `scope` and `imports` rather than the
supplying class's. Not a multibody matter, and worth fixing on its own.

### `Connections.rooted` answered by depth, not by being a root

Done as a standalone correctness fix, exactly as the panel scoped it -
2 models touched, 0 run, floors unmoved at 819 and 363. The point was
never runs: a mutation making `rooted` always `false` passed all 438
parser tests, so the behaviour was guarded by nothing.

`rooted(a)` now answers which end of `a`'s branch the roots settled
above: the node/edge build inside `choose_roots` was extracted to a
shared helper, a `graph_depths` BFS walks from the chosen roots along
the same edges, and `rooted(a)` is `depth(a) < depth(b)` for the
single branch that names `a` first. A node the graph never held, or
one first in no branch or in more than one, has no answer and is
refused by name where the graph is still whole - not answered `false`,
which would be the guess the invariant forbids. The old test at
connections.rs:327 was blind, asking `rooted` of the root itself; it
now asks it of a node with a real branch answer, and a new test shows
`rooted(b) = true` where `isRoot(b) = false`, red without the fix.

### Fresh census: the top is C-fluid, larger than thought

With the multibody line parked, the top of the refusal census decides
the next work. Folded by family rather than read line by line:

- **cannot evaluate parameters: 62**, and **57 of those are media** -
  a constant whose value needs a function run over a state,
  `h_default = specificEnthalpy_pTX(...)` and its kin. This is the
  largest named family by a wide margin, bigger than the panel's
  earlier count of 25.
- **unknown variable: 47** - the run-half aggregate-name walls among
  them.
- **structurally singular: 33** - what the differentiation shifts left.
- **cannot differentiate: 29**, **diverged/singular: 16**, **unknown
  function: 14**.

So C-fluid is the queue's head, and its kind is known: a constant
whose value is a call on the medium's own function over a state the
model has not run yet. The interpreter that would settle it already
exists one wall further on, where every deferred parameter settles at
initialisation. The next shift starts there with a small model, not
the corpus.

## C-fluid, the spec read: a parameter settles at initialisation, and a sibling folded

The shift opened where the last left it: is a media constant's value
_obliged_ at translation, or only wanted by initialisation? The
chapter answers. Section 4.4.4: a **parameter** "has a value determined
at initialization"; a **constant** is "unaffected even by the
initialization problem." And 4.4.3 blesses the exact victim as legal:

```modelica
parameter Medium.SpecificEnthalpy h_start = if use_T_start then
    Medium.specificEnthalpy_pTX(p_start, T_start, X_start) else Medium.h_default
```

`h_start` in `PartialTestModel` is a **parameter**, not a constant. Its
value is due at initialisation, where a full interpreter with loops is
legal - and that interpreter already runs: `der(z) = dsolve(16.0) - z`
walks a `while` to convergence today. So refusing `h_start` because
`h_default` has no _translation-time_ value is the compiler stricter
than the language, a fourth entry in the genre above the Evaluate,
fixed, and clock trades.

### What the small model showed, and what it did not

`dsolve(16.0)` called **directly** as a parameter binding folds to 4:
the `while` executor runs at translation when the trip count is
decidable (`statements.rs:300`, up to `MAX_WHILE_ROUNDS`). But the same
call reached as a **package constant** - `h_default = dsolve(target)` -
refused. The reason was one link: the call reached the fold with
`target` still a bare name. `target` is a sibling constant of the same
package, and the road that substitutes a constant's binding
(`class_constant_at`) resolves the call under the package's scope,
where a bare `target` is not looked up as one of the package's own
constants. Left unfolded, `dsolve(target)` reached the flat model with
an argument nothing declares, and the parameter reading it was refused.

The fix folds the package's already-settled constants into each binding
before it is walked: `dsolve(target)` becomes `dsolve(16)` and the loop
runs to a number. It is guarded by a sim test that fails on the parent
and a small model, and it is necessary - without it, wh1 (constant
`dsolve(target)`) and wh5 (a `replaceable package` medium whose
`h_default` iterates over `p_default`, `T_default`) both refuse; with
it, both fold.

### Why the corpus did not move

The floors held at 819 and 363 with no shuffle of the running list.
DryAir1 is untouched, and the reason is precise: its `h_default` fails
**earlier** than the sibling argument, on record resolution.
`why ...Air_pT h_default` reports `PartialMedium.setState_pTX answers
with ThermodynamicState, which declares no fields: it is a record kept
for another to redeclare, and the redeclaration did not reach here`.
The medium redeclares that record whole in `Air_Base`; the inherited
body of `specificEnthalpy_pTX` calls `setState_pTX` bare, and the
redeclaration is not reaching that call. A synthetic mirror of the full
shape - interface with an empty `ThermodynamicState`, `Air_Base`
redeclaring it whole and the two functions, `Air_pT` extending
`Air_Base`, a model naming `Medium = Air_pT`, the `X[:]=reference_X`
default argument, a `while` in the state-builder - folds to the right
number (`/tmp/rr4.mo`, `/tmp/wh5.mo`). The corpus does not, on some
detail the synthetic still lacks: the record is redeclared at more than
one level, its fields carry `stateSelect` modifiers written on other
constants, and `reference_X` is an array sized by `nX = size(...)`.

So this shift shipped the sibling-fold as an honest cousin - a real
red-to-green on a faithful small model, no regression, floors held -
and named the corpus's true blocker for the next shift: the redeclared
record that does not reach the inherited body which builds it. That is
a resolution defect in the redeclare road, not a strictness the spec
lifts, and it is where the 57 media wait.

## C-fluid, the wall named: a field of a record constant built by modifiers

The previous section closed on the wrong wall. It reported the corpus
blocker as a redeclared record that does not reach the inherited body -
read from `why ...Air_pT h_default`, which prints
`setState_pTX answers with ThermodynamicState, which declares no
fields`. Probing the **simulate** path, not the `why` path, told a
different and truer story.

`simulate` on the medium reaches `setState_pTX` fine: it reduces to
`airBaseProp_pT(p, T)[3]`, an indexed record-returning call. The record
_does_ reach; the `why`-path error was a red herring from a different
route. What does not reduce is the density iteration inside, and the
reason is one bare name.

`airBaseProp_pT` calls `Inverses.dofpT`, a Newton `while` that starts
from `d := p/(R_s*T)`. Instrumenting the loop head showed it running
exactly one round and then failing to settle its condition, with `d`
bound to `100000 / (R_s * 293)` where **`R_s` was still a `Ref`**. The
gas constant had not folded. With `d` symbolic every later round is
symbolic, `found` never decides, and the loop is refused as
undecidable - which upstream reads as `h_default` having no value.

`R_s` is written
`constant FundamentalConstants Constants(R_s = 287.117, MM = ..., ...)`:
a record component with `constant` variability whose fields are set by
a **modifier list**, not by an equation. Reading `Constants.R_s` splits
into head `...Basic.Constants` and field `R_s`, and the head is looked
up as a class - which it is not. It is a component. So the constant
road bailed to its `expr.clone()` fallback and the name travelled bare
into the run.

The fix is a new fallback in `class_constant_at`: where the head is not
a class, take it apart into the package that holds the component and
the component's own name, find the record-valued constant there, and
read the field off its modifiers - or off the record's own declaration
default where the modifier list leaves it out. Verified against the
corpus: `...Basic.Constants.R_s` folds to 287.117, and `MM`, `rhored`,
`Tred`, `pred`, `R_bar` all fold too, where the parent refused every
one.

### Still one wall on, and it is named too

The floors did not move: 819 and 363, no shuffle. `dofpT` still does not
fold, because one wall on there is another. With `R_s` folded, the loop
now reaches `f := Basic.Helmholtz(d, T)`, and `Helmholtz` inlines to a
record whose fields are left as unreduced `Bin` trees rather than
numbers - `getf`'s read of `f.f` comes back as `Helmholtz(...)[6]`
standing, an index into a call the run cannot walk to a scalar. The
record-returning function inlines, but its fields are not folded on the
way out, so a field read of the result does not resolve.

So the shift shipped the record-constant field as a real corpus-chain
unblock - guarded by a sim test that fails on the parent and the small
model, floors held, and demonstrably folding what the parent refused -
and named the next wall precisely: a record-returning function whose
inlined fields are not reduced to numbers, so an index into the result
stands. That is the next shift's first probe, and it is concrete: fold
the fields of an inlined record before a field read is asked of it.

## C-fluid, the next wall probed: the depth guard fires on finite numbers

The probe named above was run this shift, and it went two levels deep
before hitting a wall that is not about records at all.

Probing the running compiler: `dofpT`, the air density `while`, refused
because its loop body calls `Basic.Helmholtz(d, T)`, which inlines to a
record whose fields are enormous but numeric. Carrying that record whole
(an `expand` over the array of its fields) runs past `MAX_DEPTH` and
throws `NO_BOTTOM`, so the whole call is left standing, and a field read
of it becomes `Helmholtz(...)[6]`, an index the run cannot walk.

A narrow fix was tried: in the `NO_BOTTOM` fallback of the record-carry,
fold the numeric subtrees of what was built before carrying, so a field
that is a number collapses to a digit and the depth is gone. It works
for that path - corpus `dofpT` folds standalone, verified red-to-green
(parent refuses `Inverses.dofpT(100000, 293, 1e-6)`, the fix folds it to
1.189), and all ten suites stay green. But it was reverted, for three
reasons that are the finding:

1. **No corpus outcome.** `dofpT` folding does not move a model. One
   wall on, `airBaseProp_pT` calls `dofpT` at depth 15, and there
   `Helmholtz` hits the depth guard _inside its own body walk_ -
   `worked_body` throws `NO_BOTTOM` before it ever returns an array, so
   the fallback fold has nothing to fold. The refusal is the same
   family one level deeper, and the narrow fix does not reach it.

2. **No hermetic guard.** A 40-deep single-field expression
   (`r.f := (((x*1.01+1)*1.01+1)...)`) reproduces the deeper case in
   twenty lines and fails _with and without_ the narrow fix, both on
   `worked_body`'s own `NO_BOTTOM`. It is a clean repro of the real
   wall, and the narrow fix does not touch it - which is the proof the
   fix was aimed at a symptom, not the cause.

3. **The cause is the depth guard on finite arithmetic.** `MAX_DEPTH`
   is 32. A media property is a polynomial dozens of terms deep over
   constants - a finite number the compiler could compute - and the
   walk refuses it for being deep, exactly as the limit of thirty-two
   once guarded the stack under the name of a loop and cost five table
   models. The real fix is to fold numeric subtrees _during_ the walk,
   so depth accumulates through un-inlined calls but not through
   settled arithmetic; a number is depth zero however it was written.
   That is a change to how `worked_body`/`expand` count depth, and it
   must be measured against the table models the last depth change
   cost, not shipped narrow.

So the shift did not ship on this family: one narrow fix built,
verified, and reverted for reaching a symptom without an outcome or a
guard. What it leaves is the wall named exactly - the depth guard
counts settled arithmetic as depth - and a twenty-line repro
(`/tmp/deep5.mo` in spirit: a record field forty operations deep, read
after inline) for the next shift to fold against.

## C-fluid, the depth fold measured: a boundary fold costs six models

The fix the last section pointed to - fold numeric subtrees so the
depth guard does not fire on finite arithmetic - was built at the one
place it belongs and measured against the whole library. It works for
the media case and it regresses six other models, which is the finding.

Built narrowly: at the two depth guards that refuse an expression for
being deep (`resolve` in names.rs, `expand` in arrays.rs), and only at
the boundary itself - `if depth > MAX_DEPTH`, try `const_eval`/
`settled_by` and return the number if it settles finite, else refuse as
before. Nothing shallower folds, so a parameter written on its own name
stays tunable and the recursion-unrolling test that the eager version
broke stays green. All ten suites pass. `deep5` (a record field forty
operations deep) and the corpus `dofpT` both fold, red-to-green.

Then the library measured 813 flatten against 819, down six:
`PrismaticConstraint`, `RevoluteConstraint`, `UniversalConstraint`,
`PlanarFourbar`, `ModelicaTest.Fluid.TestUtilities.Test01RegFun3`,
`ModelicaTest.Math.Random.TestSpecial`. These are the models that
_relied_ on the guard leaving a deep expression standing for the run to
walk. Folding it to a number at the boundary took a value the run was
going to compute itself and fixed it early, or fixed it wrong, and the
model that flattened before did not. This is exactly the class of
regression the limit of thirty-two cost the last time it moved, and it
is why that limit was left alone.

So the boundary is the wrong discriminator: `settled_by` returning a
finite number does not tell a media polynomial that _should_ fold from
a multibody expression that _should_ stand. The two are the same shape
to the depth guard, and telling them apart needs something the guard
does not have - which body asked, or whether the run will walk the
result. Reverted whole: floors back to 819/363, six models restored,
ten suites green.

The finding for the next shift, now measured rather than guessed: the
depth guard cannot be turned into a fold at its boundary without costing
six models, so the media case has to be answered before the guard is
reached - where the record body is built - and only for the bodies that
have no run behind them to walk what stands. That is a narrower door
than "fold deep numbers", and it is the one still to find.

## C-fluid, the depth fold gated: the parameter mark is the discriminator

The door the last section left to find was there already. The fold at
the depth guard is right; what it lacked was the gate telling a media
polynomial that should fold from a multibody expression that should
stand. That gate is `SETTLING_PARAMETER`, the mark the flattener
already sets while it works out a parameter's value - and its own
comment says why it is the one: "a parameter's value, where a number is
the whole of what is wanted." A parameter settles to a number with no
run behind it; an equation's deep expression is meant to stand for the
run to walk. The mark is exactly that distinction.

Gated on it, the boundary fold in both guards (`resolve`, `expand`)
folds a deep-but-finite expression to its number only while a parameter
is being settled. Measured against the whole library: 819 flatten, 363
run, the six models the ungated fold cost all restored, no shuffle, ten
suites green. A twenty-line model - a record field built by a forty-
round accumulation, read by a parameter - is the guard: it refuses on
the parent (`build(2)[1]` stands) and folds to its number here, and a
sim test carries the same case.

This is real infrastructure: the air density iteration `dofpT` now
folds standalone, red-to-green, where the parent refused
`Inverses.dofpT(100000, 293, 1e-6)`. It does not by itself move a media
model, because one wall further on `airBaseProp_pT` still stands - its
call to `dofpT` passes `delp = iter.delp`, and `iter.delp` (a field of
a record-class alias) arrives at the nested body as a bare name that the
deep context did not fold. That is the same record-field failure as
`R_s`, in the alias case rather than the component case, and it is the
next wall, named and small.

So the shift ships the gated fold - guarded, measured, no regression -
and the C-fluid chain now folds two of its three deep walls (the record
constant field, the deep numeric parameter). The third is a record-alias
field read in a nested body, and it is where the next shift starts.

## C-fluid, the third wall pinned: a record-alias constant misqualified

With the deep numeric parameter folding (previous commit), the media
chain's last wall is a single unresolved name, and this shift pinned it
exactly without yet fixing it - because no small model reproduces it and
chasing it blind is how the depth fold cost six models.

The air density iteration `dofpT` now folds standalone. Inside the
corpus it is called by `airBaseProp_pT` as
`Inverses.dofpT(p=p, T=T, delp=iter.delp)`, and `iter.delp` is the one
name the while condition cannot settle. `iter` is a record-class alias -
`record iter = Inverses.accuracy` - declared in `Air_Utilities`;
`accuracy` holds `constant Real delp = 1E-1`. Reading the value two
ways proves the chain is otherwise whole: `...Air_Utilities.iter.delp`
folds to 0.1, `...Inverses.accuracy.delp` folds to 0.1, and
`dofpT(100000, 293, ...Air_Utilities.iter.delp)` with the field passed
by its full path folds to 1.189. Only the bare `iter.delp` written
inside `airBaseProp_pT` does not.

The refusal names it `...Air_Utilities.Inverses.iter.delp` - and there
is no `iter` in `Inverses`; the alias is one level up, in
`Air_Utilities`. So the fault is a name qualified wrong: `iter.delp`
written in a body of `Air_Utilities` and worked out while that body is
inlined is being qualified against `Inverses`, the package the call it
sits in belongs to, rather than climbing to `Air_Utilities` where the
alias is declared. Resolved by full path it folds; resolved bare in the
inline it lands on a package that has no such name.

No small model reproduces it: every synthetic with the same shape - an
alias in a parent package, its constant read bare from a body that a
child package's call is threaded through - climbs correctly and folds.
The corpus differs in some detail the synthetics do not yet carry, and
finding it needs corpus-direct instrumentation of where a bare
`iter.delp` is qualified during inline, not another guess. That is the
next shift's first probe, and it is the last wall between the media
constant and a number: two of the three deep walls now fold, and this
one is a misqualified name, not a depth or a record-build.

## C-fluid, the third wall traced: resolve does not fold a dotted constant

The wall pinned last commit was traced to its mechanism this shift, and
a fix was built, measured clean on the whole library, and reverted for
having no automated guard - the same discipline the depth fold taught,
one turn later.

`iter.delp` fails because `resolve` - the pass that works out the
arguments a call was written with - does not fold a dotted name that is
a constant of a class. Its `Ref` arm folds a loop variable and leaves
everything else the name it was (names.rs). A media function is handed
`delp = iter.delp`, and left a bare name it travels into the run where
the `while` that reads it cannot settle its trip count.

The fix is four lines in that arm: a dotted name that `class_constant_at`
answers is folded to its value, gated on `SETTLING_PARAMETER` so a
constant read in an equation keeps its name and its unit still checks -
ungated it broke two unit tests, the same parameter-versus-equation
split the depth fold turns on. Measured: the corpus `airBaseProp_pT`,
`specificEnthalpy_pTX`, and `dofpT` all fold red-to-green
(`airBaseProp_pT(100000, 293)` gives rho 1.189, `specificEnthalpy_pTX`
gives 19974), 819 flatten and 363 run unchanged, no shuffle, ten suites
green.

Why it was reverted: no automated guard. Every synthetic of the shape -
an alias in a parent package, its constant handed as a named argument to
a while-function threaded through a child package - folds `iter.delp` in
an earlier pass and never reaches `resolve` at all, so the fix never
fires on them and cannot be the thing they test. Instrumentation shows
why: in the corpus the name reaches `resolve` under
`airBaseProp_pT`'s scope, while in every synthetic it is folded by
`substitute_at` first. The difference is depth - the corpus body is
worked out far enough down that the constant pass has been bypassed and
the argument falls to `resolve` - and reproducing that hermetically
compounds with the depth guard the previous fix already turns on.

So the fix is real and measured clean, but it moves no full model on its
own (the redeclared-record wall is still ahead) and nothing automated
would catch it rotting, because the models it helps do not flatten yet.
The next shift lands it one of two ways: find the depth-routed repro
that sends `iter.delp` through `resolve` in twenty lines, or land it
together with the redeclare fix so a media model flattens and becomes
the guard. The mechanism is now known to the line; what is missing is
the test, not the understanding.

## C-fluid, the parameter wall broken: the constant is handed to the run

The previous note reverted the `iter.delp` fix for having no guard. This
one lands it, with a second fix beside it and a hermetic guard for both,
and the C-fluid parameter barrier - the reason this queue was named -
falls: the media `h_default` refusals go from 62 to 0.

Two fixes together, each small:

`resolve`, the pass that works out a call's arguments, now folds a
dotted class constant while a parameter is being settled (names.rs), so
`delp = iter.delp` handed to the density iteration becomes a number
rather than a bare name the while cannot settle. Gated on the parameter
mark, the same split the depth fold turns on, so an equation's constant
keeps its unit.

`substitute_at`, where a constant is read, now hands a parameter the
constant's binding when the constant road could not fold it to a number
(constants.rs). A medium's `h_default = specificEnthalpy_pT(p, T)` has a
body that iterates past the depth the constant fold follows, so
`class_constant_at` returns nothing and the name reached the parameter
bare. Handed the binding instead - with the sibling constants folded
into its arguments - the parameter's own deeper walk, which folds a deep
numeric field while a parameter settles, works it out. Gated the same
way, because a run has to be behind the value to walk what stands.

Measured: the media `h_default` barrier, 62 models that refused `... .
h_start asks to be evaluated before the run`, is gone. DryAir1's
`h_start` now evaluates, and the model advances to a body-level wall
(`unknown variable volume.medium.state.h`, a ThermodynamicState field
the equations read that the record layer has not expanded). The flatten
and run floors do not move - the models that cleared the parameter wall
stop at the next one, and the ones that ran still run - so the win is in
the census, not the counts: 819 flatten and 363 run unchanged, no
shuffle, ten suites green.

The guard is hermetic this time. A fifty-line model carries the whole
shape: a `replaceable package` medium whose `h_default` is
`enthalpy(setState(p, T))`, the medium redeclaring `setState` to build a
state whose enthalpy divides by a density solved with a Newton `while`
over a nineteen-term polynomial deep enough that the constant road
cannot fold it. Read by a parameter it is refused on the parent and
folds here, and a sim test carries the same case. What no synthetic
reached last shift, this one does, because it puts the depth past the
guard where the corpus puts it.

The remaining wall is the record field the model body reads -
`state.h` where `state` is a `ThermodynamicState` the medium redeclared
whole - and it is the last one between these media and a run. The
parameter barrier that named this queue is closed.

## C-fluid, the record instance filled: a state gets the medium's fields

With the parameter barrier closed, DryAir1 stopped at a body-level
wall: `unknown variable volume.medium.state.h`. This is the record
instance one, and it was a four-line fix once found.

`BaseProperties` holds a `ThermodynamicState state`, and its equations
read `h = state.h`. `ThermodynamicState` is empty in the interface and
redeclared whole by the medium - the same empty-placeholder record the
function layer already knew to look past. But a component of that type
was instantiated straight from the interface: resolving `state`'s type
landed on the interface's empty record, so the instance came out with
no fields and `state.h` named a variable nothing declared.

The instantiation already holds the mark of the name the type was
reached by - `AskedAs::resolving`, set so a body written in the base
finds the medium's functions. The fix reads the record under that mark
before instantiating it, through `record_asked_under`, the same call
the record-building layer uses. Found under the medium, the record has
its four fields, and `state.h`, `state.p`, `state.T`, `state.d` become
components.

Measured: the `state.*` unknowns go from 12 to 1, and the `unknown
variable` barrier from 31 to 22 - nine kinds cleared. DryAir1 advances
again, now to `shortPipe.port_b.Xi`, a mass-fraction wall further into
the model. Floors unchanged (819/363, no shuffle, ten suites green):
the models that cleared this wall stop at the next, and the win is the
census. Guarded hermetically by a flattening test - a redeclared empty
record whose instance's field is asserted to be a component - and the
small model it came from, both refused on the parent.

The media are being walked into their bodies now, one wall at a time:
the parameter that named the queue, then the state record it reads,
then the mass fractions. Each is a kind cleared and a barrier named,
and none has cost a model yet.

## C-fluid, the empty mass fraction: a zero-length connector field

The state record filled, DryAir1 stopped at `unknown variable
shortPipe.port_b.Xi`. The fluid port carries `Xi[nXi]`, the independent
mass fractions, and dry air is one substance with `nXi = 0` - so the
port has no `Xi`, and the connection that would equate it names a
variable nothing declares.

The potential equalities a connection writes are one per member of the
connector, and a member with dimensions is written whole rather than
element by element. `Xi[0]` has a dimension, so it was written whole as
`port_b.Xi = port.Xi` - and a zero-length array is no scalar the flat
model carries. The fix skips a member the flat model does not have,
neither as a scalar nor as any element, the same test the unconnected
flow of a set of one already makes before it forces a name to zero.

Measured: the `unknown variable` barrier from 31 to 20 across the two
record fixes, and DryAir1 advances again - now to `subscripts and
arrays survive flattening only as scalars`, an array wall deeper in.
Floors unchanged (819/363, no shuffle, ten suites green) on a core
connection path. Guarded by a sim test and a small model - a connector
with a `Xi[0]` field, connected, refused on the parent and run here.

Three walls into the media bodies now, each a kind cleared and none a
model lost: the parameter, the state record, the empty mass fraction.
The next is an array one, and the media are still walking toward a run.

## C-fluid, the array wall named for next: X[nX] not expanded

Three body walls cleared, DryAir1 stops at the fourth, and it is named
here for the next shift rather than fixed, so the shift ends on a clean
tree.

Probed: the refusal is `subscripts and arrays survive flattening only
as scalars: volume.medium.X subscripted by [an expression of more than
one number]`, and the equation behind it is `volume.medium.X[1] = 1`.
`volume.medium.X` is declared nowhere - the array `X[nX]`, the full
mass-fraction vector, did not expand into its element `X[1]`, so the
subscript reaches the run as an `Index` into a name the flat model does
not carry.

`nX` is `nS = size(substanceNames, 1)`, one for dry air. The dimension
is a constant the medium counts, and if it did not fold to one during
instantiation the `X` component measured no length and produced no
element - the same shape as `reference_X` and the trace-substance
counts the constant road already learned to settle, but here at the
point a component's dimensions are measured rather than a constant read.
The next shift starts at where `instantiate_components` measures a
component's dimensions, with `volume.medium.X` the probe: whether `nX`
folds to one there, and if not, why the count the medium states does
not reach the array it sizes.

The media are four walls into their bodies, each cleared with a guard
and no model lost, and this is the fifth, named and small.

## C-fluid, the fifth wall was an assert: an index in a loop check

The array wall named last commit was not the array itself but a check
over it. `X[nX]` expands fine; what stood was `X[i]` inside an assert.

The media guard their mass fractions with `for i in 1:nX loop
assert(X[i] >= 0 and X[i] <= 1, ...) end for`. The loop unroll folds
the loop variable into an equation's sides and sends them through the
array layer, so `X[i]` becomes `X[1]` - but the assert branch of the
same unroll folded the variable and stopped, never expanding the
condition. So `X[1]` stayed an index into the whole `X`, reached the
run, and the model was refused for a subscript that survived
flattening. The fix expands the assert condition the same way the
equation is expanded, one round's `X[i]` becoming that round's element.

Measured against MSL 4.1.0: DryAir1 clears the subscript wall and now
reaches its equations - it stops at `algebraic loop diverged`, a
numerical failure in the solve rather than a barrier in the flattener,
which means the model is being simulated. Floors unchanged (819/363, no
shuffle, ten suites green) on a core loop-unrolling path. Guarded by a
flattening test that asserts the check reads `X[1]`, refused on the
parent, and the small model it came from.

Five walls into the media bodies, and DryAir1 is now in its own
equations: the parameter, the state record, the empty mass fraction,
the array expansion, and the check over it. What is left is a numeric
one - an algebraic loop that needs a start the media do not give - and
that is a different kind of work from clearing a barrier.

## C-fluid, into the numerics: a pressure that starts at zero

Five flattener walls cleared, DryAir1 is in its equations, and the
sixth wall is the first that is not the flattener's: `algebraic loop
diverged: [volume.medium.T, ..medium.p, ..]`. It is diagnosed here for
the next shift, not fixed, because it is numeric and its cause is one
isolated defect worth its own change.

The loop couples the medium's `T`, `p`, `d` through `airBaseProp_pT`,
which solves density over Helmholtz with a Newton of its own. The outer
Newton needs a start near the answer, and it has one for `T` (288.15)
and `d` (1.0) - but `medium.p` starts at zero, which is unphysical, and
a density solved at zero pressure runs away. The start is there in the
model: the medium writes `extends PartialPureSubstance(AbsolutePressure
(start = 1e5), Density(start = 1.0), ...)`, giving the type of `p` a
start of a hundred kilopascal. That start does not reach the component.

Probed to the line: `medium.p` is the `p` of `BaseProperties`, and its
type is resolved under the scope `PartialMedium.BaseProperties` - the
interface that declares the model - where `AbsolutePressure` carries no
start. The medium's `AbsolutePressure(start = 1e5)` is a modifier on
the type in the medium's own `extends`, in view only under the medium's
scope. The same probe shows `medium.state.p`, resolved under
`Air_Base.ThermodynamicState`, does get the 100000: the record field is
resolved under the medium, the model field under the base. So it is the
asked-under question again, one layer over from where it was answered
for functions and records: the type of a component in an inherited
model has to be resolved under the medium the model was reached through,
not the interface that wrote the model, or the attribute the medium set
on that type is out of view.

So the next shift carries the asked-under scope into `resolve_type` for
a component of an inherited model, the way it is already carried into a
body written in a base and a record kept empty by one. With `medium.p`
resolved under the medium, `AbsolutePressure` has its `start = 1e5`, the
pressure starts at a hundred kilopascal rather than zero, and the
algebraic loop has a start it can converge from. DryAir1 may then be the
first of the media to run - the first the run count has moved for since
this queue was named. The barrier is numeric; its cause is a type
resolved under the wrong scope, the third face of a family already
twice cured.

## C-fluid, the numeric wall probed deeper: not one pressure but three

The pressure-at-zero diagnosis was probed with a fix this shift, and the
fix taught the wall is wider than one start. Recorded so the next shift
does not repeat the half-measure.

The algebraic loop's Newton takes its guess from `algebraic_start`,
built from each variable's `start` attribute. An initial equation says
more: `volume.medium.p = volume.p_start` starts that pressure at a
hundred kilopascal. Feeding the initial equations that fold with the
parameters into the guess gave `volume.medium.p` its 101325 and
`volume.medium.T` its 293.15 - measured, they arrive.

But the loop still diverged, and the reason is the other two mediums.
`ambient.medium.p` and `fixedMassFlowRate.medium.p` have no initial
equation of their own - they are pure algebraic variables the
connection sets equal to the volume's - so nothing seeds them and they
start at zero. The medium's density solved at zero pressure runs away
before the outer Newton can make the three pressures agree. The
initial-equation guess is right and not enough: it seeds the ports that
state their own start and leaves the ones that inherit it through a
connection at zero.

So the complete fix seeds every medium pressure, not just the ones with
an initial equation - from the medium's `p_default`, the constant every
medium carries for exactly this, or by propagating a connected set's
guess from the member that has one. The former is the media's own answer
to "what pressure to start at" and reaches every port; the latter is
general but needs the connection sets in view where the guess is built.
The next shift picks one, with DryAir1 the measure: three pressures
seeded, the loop converges, and the first media model runs. The fix
tried this shift - initial equations into the guess - was reverted for
seeding only one of the three and moving no model.

## C-fluid, the numeric wall was never the guess: a loop over a black box

The last two notes chased the algebraic guess. This shift disproved that
whole line: forcing every pressure to a hundred kilopascal, every
temperature to 293, every density to 1.2 - a physical start for all
three mediums - the loop still diverges. The guess was never the wall.

What the loop is: `h = airBaseProp_pT(p, T)[3]` and `d = airBaseProp_pT
(p, T)[7]`, with `p` and `T` set through the connections, and
`airBaseProp_pT` a function whose body solves a density with a Newton
`while` of its own. Evaluated as a parameter it is exact and
deterministic - `airBaseProp_pT(101325, 293).h` is 19971 every time. It
is only inside the outer solve that it fails.

So the wall is the one this project has not met before: an algebraic
loop whose residual runs an iterative black box. The outer Newton builds
its Jacobian by finite differences - perturb `p`, re-run the whole
nested iteration, read the change - and a nested `while` that stops on a
tolerance does not move smoothly with the perturbation: two nearby `p`
give densities converged to different last steps, the difference quotient
is noise, and the Jacobian it builds points nowhere. A good start does
not save a Newton whose derivative is wrong.

This is numeric-analysis work, not a barrier to clear or a scope to fix.
The routes are known and none is small: give the media functions
analytic derivatives so the outer Jacobian is exact rather than
differenced through the iteration; or tighten the inner tolerance far
below the outer step so the black box looks smooth to it; or solve the
loop with a method that does not need a Jacobian at all. The next shift
that takes DryAir1 the last step chooses among those, and it is a
different kind of shift from the seven that cleared its flattener walls.
The barrier is named truly now: a loop over a black box, and the guess
was a red herring the measurement caught.

## The register at 819/367, and the largest name in it

A census taken before any code, both halves from one pipe, so that the
work of the shift was chosen by the ranking rather than by interest.
The rows are added by meaning first, because the counter splits a
family by its wording: the run half's kinds come to 452 refusals over
452 models, and the families behind them are

| family                      | models |
| --------------------------- | -----: |
| unbalanced model            |    167 |
| a parameter without a value |    114 |
| singular, of both kinds     |     46 |
| unknown variable            |     37 |
| an algebraic loop           |     34 |

The parameter row is two wordings of one illness - `cannot evaluate
parameters` at 52 and `parameter X has no value` at 62 - and read as
separate rows it would have ranked below the loops. The singular row
is the same trick: 33 structurally singular and 11 with a singular
Jacobian, which are not one cause but are one question.

None of those is where this shift went, and the reason is worth
keeping. A row is a wall and not a family, so the rows were probed
before one was chosen, and the probe found something the ranking hides:
inside `unknown variable`, 37 models over fourteen distinct names, one
name carries eighteen. `fluidConstants[1].molarMass` is the largest
single cause in the run half of the register - larger than any
unbalanced figure, which scatter across chapters - and it sat in a row
ranked fourth.

The lesson is the one already written about counting kinds, seen from
inside a row rather than across two: the count of kinds is a lower
bound on the families, and a single row may hold one enormous cause and
thirteen singles. Ranking chose the wrong work here; ranking plus a
probe of the row's contents chose the right one.

## What eighteen models stood on: a value written on the declaration

The name resolved nowhere because the value was never gathered. A
medium's fluid data is written

```modelica
constant FluidConstants[1] waterConstants(
  each molarMass = 0.018015268, each criticalPressure = 22064.0e3, ...);
```

which is a record constant with no binding at all - every field given
by a modifier of the declaration - and it is reached from the medium
through `extends PartialTwoPhaseMedium(fluidConstants = waterConstants)`,
where the name belongs to `Modelica.Media.Water`, the package the medium
is written _inside_ rather than one it extends.

Two independent gaps, either alone enough to lose the number. The
gathering asked a component only for its binding, so a declaration
saying everything through modifiers was carried with no value; and the
hop from an `extends` modifier to the constant it names read only the
gathered basket, which walks bases and never parents. Both are fixed:
a record constant reads as the constructor its modifiers describe, and
the hop falls back to the enclosing packages.

Shrinking the real model rather than growing a synthetic one is what
found the second gap. The first synthetic model - a record constant
with modifiers, read through an `extends` - passed with only half the
fix in place, because the constant and its reader sat in the same
package. Only when the constant was moved to the parent, which is where
the library actually puts it, did the model go red again. A synthetic
model tests the layer you imagined, and the parent-package hop was not
it.

Measured on one binary, before and after: 819 flatten and 367 run
either way, the run and flatten lists identical line for line. All
eighteen models cleared the wall and stopped at five different ones -
`h_default` unresolved, `inStream` unknown, an unbalanced count, a
`bpro` subscript, a differentiation that will not go through. So this
is an entry emptied rather than a count moved, and the eighteen are
five families now, not one. The next shift that wants them should
probe those five before ranking them, on the evidence just above.

## The diverged loops are five families, not one

`algebraic loop diverged` counts 18 and reads as a single wall. Probed,
it is five, and the two the media queue cares about are the smallest
part of it: DryAir1 and DryAir2 are 2 models, and the black box behind
them is the one already diagnosed. Six are FluidHeatFlow, whose
`TwoPort` writes `flowPort.H_flow = semiLinear(m_flow, h_a, h_b)` and
whose loops are over that zero-slope meeting point; five are Rotational
friction, diverging on `clutch.tau` and `brake.tau`; three are
Electrical and one Magnetic.

Recorded because the media queue was about to take DryAir1 on the
strength of the row: two models is not a shift's work, and the
`semiLinear` six are a different and larger cause wearing the same
words.

## The singular rows probed, and one of them is already answered

The other two rows the queue had not explored, probed the same way, so
that the next shift is not the third to look at them from the outside.

`structurally singular` counts 33 and is three things. Twenty-two
cannot differentiate through an algebraic variable, and fourteen of
those are MultiBody, which has a panel of its own and is the largest
untouched family in the run half after the unbalanced. Five are an
equation that constrains no state, where index reduction has nothing to
work with. And six cannot differentiate `abs`.

That six is worth naming because it looks like free work and is not.
The obvious rule - `der(abs(x))` is `sign(x) * der(x)` - is the one
this project measured and refused, since it is wrong at exactly zero
and zero is where these models live: a flow reversing is the whole
subject of `TestRegRoot2Derivatives`. The rule that replaced it says
nothing about `abs` at all, and these six are what it does not reach.
Taking them means an honest derivative for a function with a corner,
not a special case, and a shift that opens it should say so before it
starts rather than discover the refusal is deliberate.

The remaining eleven of the 46 are a singular Jacobian in an algebraic
loop, which is a numeric row and not a flattener one.

## The eighteen split, and a chain of three under them

The eighteen models that cleared `fluidConstants` last shift were
probed one by one, which the shift before had asked for and not done.
They are not one wall but five: eight at `unknown function inStream`,
three at `unknown variable Medium.h_default`, four unbalanced, two at
`bpro[5]`, one structurally singular. The largest was taken, and
behind it stood two more of the same kind, so the three were walked as
a chain rather than reverted one at a time.

**`inStream` under a subscript.** The walk that rewrites `inStream`
into its connection set's mix was written out by hand, variant by
variant, and stopped at the ones nobody had named. A subscript was
among them, and every `inStream` a medium sees arrives inside one:
`waterBaseProp_ph(p, inStream(h), 0, 0)[9]`. The second half of the
same fault was the layer deciding which connectors carry streams at
all, which read a class's own components where the fluid ports say
everything through an `extends`. Eight models, no count moved.

**A record handed straight on.** `hvl_p(p, boilingcurve_p(p))` works a
whole property record out at the call site and passes it to the
reader. Both sides of the run knew how to take an answer of several
numbers and neither knew how to give an argument of several, so the
reader's `bpro` arrived as one number and `bpro[1]` named nothing.
Two models, 367 to 369 - the first count moved in this family.

**A constant whose value only a walked body knows.** `h_default =
specificEnthalpy_pTX(...)` reduces to a call and no further. Three
gaps, each enough alone: the recipe was handed on only while a
parameter was being settled; only for a bare call, not one under a
subscript; and the bodies travelling with a model were gathered from
its equations alone, so the value settled before the run met a
function the compiler was carrying the text of and said nothing works
it out. Eight models, no count moved.

Two things worth keeping from how this went. The third fix cost a
model before it was right - gathering bodies from declarations made an
unwalkable generator refuse a whole model, and the noise generators
are written on a `startTime` no equation reads. The floor caught it at
818, and the flatten-list diff named it in one line where the totals
only said something was wrong.

And the small models lied twice. Written to test the third fix they
passed with and without it, three times running, because a body simple
enough to write in a test is simple enough to inline - which is the
blind spot this file already names, met from a new direction. What
distinguished it in the end was the shape the library actually uses: a
body answering with a vector, asked for one field of it. Reproduce
small, but reproduce the shape rather than the story.

## The unknown that cannot be solved for, and where three models went

The `singular` row's largest part - twenty-two models that cannot
differentiate through an algebraic variable - was probed rather than
guessed at, and the eight non-MultiBody ones turned out to be four
families and not one: three inverse models under `GenerationOfFMUs`,
three summing currents at a node, two transistors, two `CombiTable2D`
tests. The probe was worth a minute; a shift that had trusted the row
would have gone looking for one cause.

What the largest of them wanted was a rule the reduction did not have.
It could reach a derivative through an unknown some equation defined
outright, or one a linear rearrangement produced - and the saturating
inductor writes neither. `Psi = Linf*i + c*atan(i/Ipar)` ties the
current to the flux and nothing puts `i` alone on a side. The refusal
named the current, so what it looked like was a missing definition,
and what it was is a function that cannot be inverted.

The solution was never needed, only the derivative. With residual
`g(t, x) = 0` the implicit function theorem gives `dx/dt` as
`-(dg/dt at x fixed) / (dg/dx)`, and both halves are derivatives this
module already takes. Which is the general shape of several fixes in
this file: the compiler was asking for more than the question needed.

Two things had to move with it, and the second is the one worth
remembering. The equation offered for a name must be one no
rearrangement solves - `0 = p.i + n.i` is linear in either current, and
taken here it answers `der(p.i)` with `-der(n.i)` and goes round the
circuit for ever rather than reaching the flux. And the choice of which
state to demote walks the same equations: without that the
differentiation succeeded and the model was refused one line later for
a constraint that pins no state. A fix that goes halfway moves the
refusal without moving the model, and reads from outside like a fix
that did nothing.

The measurement is the shape the charter asks for and worth quoting as
an instance of it. Totals identical - 819 and 369 - and the run list
identical line for line. The census is where the work shows: the row
went 22 to 19, no other row lost anything, and the three are named -
`GenerationOfFMUs` twice and `TestSaturatingInductor`, now standing at
an algebraic loop and an initialisation that is not square. A wall
removed and not a count moved, so the floors stayed where they were.

And the small model lied once more, in the way this file has now
recorded three times. Written with the currents alone it passed with
and without the fix, because a model with nothing to reduce never
reaches the rule; what distinguished it was the shape the library
uses, voltages and ground included. The test asserts the voltage -
0.283618581907 at unit time - rather than that the model ran.

### What the row still holds, probed and left

The nineteen that remain were probed rather than left as a number, so
the next shift starts inside them.

The `CombiTable2D` pair wants a rule this project has already measured
and refused. `trapezoid1.T_start` is assigned by a `when`, and between
events its derivative is indeed zero - which is true, and useless: the
reduction demotes a state on the strength of that zero and has built
something that stops holding the moment the clause fires. It cost two
models when it was tried. Anyone opening this pair is taking on what a
`when`-assigned name's derivative honestly is, not the shortcut.

The two `MovingCoilActuator` models are a different shape from the
inductor and were not reachable by the same fix. `pmActuator.r.p.i` is
named by the very equation under reduction, so the rule above will not
offer it - an equation cannot define its own way out. What determines
it is the matching, and reaching it means asking the matching rather
than the definitions. Larger than a link, and honestly a separate
piece of work.

The two heating transistor models refuse on `der(T2.vbc)` - a
derivative already carried as an unknown, needing a second
differentiation rather than a first. Fourteen are MultiBody and
parked under their own panel.

## The unbalanced row surveyed: seventy-three models, seven families

The largest entry of the run half was carried forward from an old
census as "167 models, 13 of them machine-shaped". Both halves of that
were stale. Measured on the corpus at 362 models that flattened
without running and 362 that ran - two list lengths, not the
flatten/run pair - with
`scripts/refusals.sh .msl/Modelica unbalanced`, the row holds **73**
models, and the machine-shaped part of it is **14**, not 13.

The script cuts the row along two axes before anything is probed, and
the first axis is the one that matters most: the _sign_ of the miss.
Too few equations and too many are different illnesses and may not be
added together.

- **too few equations (`-`): 63 models.** The spread is narrow -
  29 at `-2`, 12 at `-1`, 8 at `-3`, 7 at `-4` - with a thin tail at
  `-19`, `-24`, `-25`, `-40`, `-80`.
- **too many equations (`+`): 10 models.** Four small (`+1`, `+2`) and
  a cluster of transformers at `+60` and `+120`.

By chapter the row is overwhelmingly Electrical (about half), then
Mechanics, then Fluid, Clocked, Thermal, StateGraph, Magnetic.

A second fact about the row, before the families: **47 of the 73 are
helper classes** - `.Utilities.`, `.BaseClasses.`, `.Components.` -
and not runnable examples. The row is therefore worth much less to the
run count than its size suggests; the 26 that are examples proper are
the part that could move a floor.

### The families, probed one model per cell

**1. The adaptor's dead derivative branch - 13 models, all `-2` to
`-4`.** `Modelica.Blocks.Interfaces.Adaptors.FlowToPotentialAdaptor`
writes `y1 = if use_pder then der(y) else 0`, and every model in this
family instantiates it with `use_pder=false, use_fder=false`. The
refusal names `der(voltageToCurrent1.y1)` as undetermined: the branch
that is _not_ taken is still generating a derivative unknown. The
conditional's condition is an `Evaluate=true` structural parameter and
is false, so the `der` should never have been minted at all. Probed on
`Utilities.Resistor` (41 equations for 43 unknowns): `why` shows the
flat equation still carrying the whole `if`, and the conditional
connectors `pder`/`pder2` correctly absent - "declared nowhere". So
the connectors were removed by the conditional-declaration layer and
the equation was not folded by the same knowledge. Same wall in
Electrical, Mechanics.Rotational, Mechanics.Translational and
Thermal.HeatTransfer, which is why it looks like four chapters and is
one class. This is the largest family and the most localised.

**2. The DC machine and H-bridge chain - 14 models, `-1` to `-3`.**
`dcpm.airGapDC.pin_ap.v`, `dcpm.internalSupport.tau`,
`hbridge.fire_n`. Probed on `DCPM_Start` (190 for 191): `pin_ap.v`
_is_ named by two equations, one inside `airGapDC` and one from a
connection, so this is not a missing equation but a matching that ends
one short. The old note's "13 machine-shaped" is this family, now 14,
and the note that it is not the same thing as the top of the census
still holds.

**3. Op-amps - 7 models, `-2` to `-5`.** `opAmp.i_s`, `opAmp.out.i`,
`n2.i` repeat across `OpAmpCircuits.{Add,Buffer,Feedback,Gain}`,
`ControlCircuit`, `DifferentialAmplifier` and `CauerLowPassOPV` (the
`-19` in the tail, and the only large one). Probed on `Buffer`:
`opAmp.i_s` has exactly one equation defining it and is still called
undetermined, so like family 2 this is the matching and not the model.
`CauerLowPassOPV` names 23 unknowns of the same shape, which suggests
one repeated component and not 23 illnesses.

**4. Classes with no equations at all - 5 models.** `BufferMain` (0
equations for 80 unknowns), `Buffer_Recipe_TBD` (0 for 40),
`Adapter_Inference`, `Adapter_Superposition`,
`ComponentsThrottleControl.SpeedControl`. `Buffer_Recipe_TBD` is a
`class` holding `Boolean S0..S14` and an actuator connector, with its
whole content in `algorithm` sections elsewhere; `why Recipe1.S0`
answers "named by: no equation of the flat model". These are not
models anybody simulates and the refusal is arguably correct. Park
them: five models of the row are noise.

**5. Too many equations - 10 models, two shapes.** The StateGraph four
(`CompositeStep`, `CompositeStep1`, `CompositeStep2`, `MakeProduct`,
`+1`/`+2`) refuse with "nothing is left for
`initStep.inPort[1].occupied = inPort.occupied`" while `why` shows
`inPort.occupied` _also_ fixed by `inPort.occupied = false`. A
connector's default and the connection equation are both being
counted, so one of the two is spurious. The transformer three
(`Rectifier6pulse` `+60`, `Rectifier12pulse` `+120`,
`AsymmetricalLoad` `+60`) and `TestSensors` (`+2`) refuse over
`star2.plug_p.pin[k].v = star2.pin_n.v`, a polyphase star generating
one equation per phase where the connection set already has them. Both
shapes are equation generation counting something twice, and the
transformer numbers being exact multiples of 60 says how mechanical
the duplication is.

**6. The expandable-bus robot - 5 models, `-2` to `-24`.**
`axisControlBus.motorSpeed`, `motion_ref_axisUsed.y`, `moving[2..24]`
in `RobotR3.Utilities.{Motor,Controller,AxisType2,GearType2,
PathToAxisControlBus}`. The `-24` one is the largest single miss after
the equationless classes. All are helper classes.

**7. Singles and small pairs - the remaining ~19.** Clocked's
`inverseBlockConstraints.y2` and `T_c`, the FluxTubes pair with
`mass.a` and `stopper_xMax.lossPower`, StateGraph's `Tank`/`valve`/
`Source` over `outflow1.open`, `Rotational.Utilities.{DirectInertia,
InverseInertia}` over `inertia.a`. Several of these have the same
shape as families 2 and 3 - `inertia.a` is named by two equations,
`outflow1.open` by one - and the charter's warning applies: a column
of ones may be one layer, and this batch has not been probed to the
layer yet.

### What the survey says about where to work

The count of kinds here is one row; the count of families is seven,
and the two ends of the gap are exactly what the charter says they
would be. Ranked by what they could actually buy:

- Family 1 is 13 models on one class and one unfolded `if`, and it is
  the only family where the cause is visible without opening the
  matching. It is also the only one whose fix is plainly local. But 10
  of its 13 are helper classes.
- Families 2, 3, 6 and much of 7 all show the same signature - a name
  with equations naming it, still reported as determined by nothing -
  which is a matching that comes up short rather than 30 separate
  faults. If that is one layer, it is the largest thing in the row by
  a wide margin, and a probe into the matching is worth more than any
  of the individual families.
- Family 5 is a different illness entirely (duplication in equation
  generation) and must not be counted with the rest.
- Family 4 should be excluded from the row's size in future readings.

No code was written for this entry; it is reconnaissance.

## The adaptor's dead branch: two layers reading one annotation

The survey's family 1 was thirteen models on `FlowToPotentialAdaptor`
and its mirror, and the cause was visible without opening the matching:

```modelica
parameter Boolean use_pder = true annotation(Evaluate = true);
RealOutput pder if use_pder "Optional output for der(potential)";
equation
  y1 = if use_pder then der(y) else 0;
  y2 = if (use_pder and use_pder2) then der(y1) else 0;
```

With `use_pder = false` the layer of conditional _declarations_ reads
the annotation and removes the connector. The `if` in the _equation_ was
not read with the same knowledge, so `der(y)` stayed standing in a
branch nothing takes, and `y2` was then owed a derivative of `y1` -
which no equation moves. `Utilities.Resistor` refused as 41 equations
for 43 unknowns over exactly that.

Two layers held the same annotation and only one acted on it. They now
read it together: an `if` expression whose condition is settled by a
parameter carrying `Evaluate = true` is replaced by the branch that
stands, before anything can be owed a derivative of the other.

The narrowing is the whole of the correctness. Folding on _any_ settled
parameter was tried first and four tests said no in one run - a Boolean
constant came through as `Number(2.0)`, and `v = if high then {1, 2}
else {3, 4}` was built one way where the declaration promised a
parameter the run could be handed again. An ordinary parameter settles
to a number that is still the run's to change; `Evaluate = true` is the
declaration saying this one is structure. Both readings measure the
same on the corpus, so the narrow one is taken on the strength of the
tests alone, which is the cheapest news this change had.

Measured from one binary, the fold behind an environment switch, two
passes over the corpus and the run lists diffed line by line:

```text
819 flatten / 369 run  ->  820 / 374
721 / 364 (runnable)   ->  722 / 369
```

Five models gained, none lost: `GenerationOfFMUs` in both Analog and
Translational, `ResonanceCircuits`, and the two `ToroidalCore` flux
tubes. Ten of family 1's thirteen were helper classes and are not
counted as examples, which is why thirteen models at the wall buys
five on the board.

`Utilities.Resistor` itself flattens further and stops one storey up:
`nothing determines v1, voltageToCurrent2.pin_p.v` - its own top-level
inputs, connected to nothing, which is the family the entry below this
one names. A wall behind a wall, as expected, and it is that family and
not this one.

## Probing the shared signature: it is two layers, not one

The survey above ranked "families 2, 3, 6 and much of 7" highest on the
strength of a shared signature - a name with equations naming it, still
reported as determined by nothing - and said that if it were one layer
it would be the largest thing in the row. It was probed on the two the
survey named, `DCPM_Start` from family 2 and `Buffer` from family 3.

It is two layers, and the number that separates them was in the refusal
all along: how many names it lists against how far short the counts are.

`Buffer` is 41 equations for 45 unknowns and names exactly four -
`n2.i`, `opAmp.i_s`, `opAmp.out.i`, `r1.n.v`. A maximum matching that
leaves as many unmatched as the deficit has found nothing pathological;
it is reporting a model genuinely short of four equations. And what is
short is visible in the source: `Buffer` extends `PartialOpAmp`, whose
connectors `p1`, `n1`, `p2`, `n2` are the block's own terminals and are
connected to nothing. Shrunk, the whole of it is twelve characters of
model:

```modelica
model Unconn2
  connector Pin Real v; flow Real i; end Pin;
  Pin p1;
  Real x;
equation
  x = p1.v;
end Unconn2;
```

which refuses with `2 algebraic equation(s) for 3 unknown(s); nothing
determines x`. A `flow` variable of an unconnected connector gets its
zero, and the potential beside it gets nothing - correctly, for a model
that is a component rather than a system. These are partial circuits
awaiting a testbench, and the run half counts them as examples.

`DCPM_Start` is the other shape: 190 equations for 191 unknowns, one
short, and it names _nine_. Nine unmatched names against a deficit of
one is not a model missing equations, it is a matching that could not
place eight it had equations for. `pin_ap.v` is named by two, `ie.n.v`
by two, `internalSupport.tau` and `powerBalance.powerMechanical` by one
apiece, and all four sit in the machine chain. This is the layer the
survey was hoping for, and it is family 2 alone.

So the signature does not name a family; the arithmetic does. **Names
listed equal to the deficit** means the model really is short and the
question is which equation was never generated. **Names listed greater
than the deficit** means the matching failed, and the excess is how
badly. That test costs nothing - both numbers are already printed in
every unbalanced refusal - and it splits the row without opening a
single model.

Which shrinks the prize. The 30-odd models of families 2, 3, 6 and 7
are not one beast; family 2's fourteen machines are, and the rest have
to be re-read with the arithmetic before anyone plans against them.

## The arithmetic applied to the whole row

The test the previous section proposed is now in `scripts/refusals.sh`,
where `unbalanced` prints how many names each refusal listed against
how far the balance misses. One pass over the library splits the row
in two:

```text
  97 honestly-short          named == the deficit
  65 matching-fell-short     named >  the deficit
```

By chapter, the two halves are not the same library. Where the
matching fell short: Electrical 25, Magnetic 19, Mechanics 12, then
Fluid 3, ModelicaTest.Fluid 3, and singles in Media, Thermal and
StateGraph. Where the model is honestly short: Electrical 33, Magnetic
21, Mechanics 9, Fluid 7, ModelicaTest.Fluid 7, StateGraph 6, Clocked
6, ModelicaTest.Media 5.

So the two questions are asked of the same three chapters, and the
work queue reads differently for each. The 65 want the matching, or
what it can reach through: the equations are there and it did not
place them. The 97 want whatever never wrote the equation, which is a
different search - and StateGraph and Clocked appear only on that
side, so they are not matching failures at all.

Family 2, the fourteen machines, was the first of the 65 to be taken,
and the finding was not in the matching: `internalSupport.tau` had no
equation because the port's zero flow was withheld. The check that
asked whether a port was already spoken for threw the member away and
kept the path, so `internalSupport.phi` appearing in
`phiMechanical = flange.phi - internalSupport.phi` silenced the torque
beside it. Two machines run for it - `DCPM_Start` and
`DCPM_withLosses`, 374 to 376, no victims - and the other twelve
moved on to the next wall rather than falling: `DCEE_Start` and its
neighbours now refuse with `algebraic loop diverged` over
`inertiaRotor.a`, and `DCPM_Temperature` with a fixed initial value of
`wMechanical`. A census entry emptied, a wall behind it named.

Which also corrects the reading above: a model counted as
`matching-fell-short` need not have a matching fault. Nine names
against a deficit of one meant the matching could not place eight, and
it could not place them because one equation of the eight-way chain
was never written. The arithmetic says the equations are probably
there; it does not say the matching is at fault. It is still the
cheaper half of the row to look at, because the excess points at a
chain rather than at a single name.

## The induction machines: a member of a sibling had no shape

The next family of the 65 was taken from the same census, grouped by
prefix: of the 25 `matching-fell-short` models in Electrical, 14 are
`Machines`, 5 `QuasiStatic`, 5 `Analog`, 1 `PowerConverters`. The
fourteen stand apart from every other model in the chapter by the sign
of the miss - all of them have _too many_ equations, `+19` to `+28`,
where the rest of the chapter is short. A surplus that large is not a
missing equation; it is a written one, written too often.

`why` named the cause in a single call, before a line was changed:

```text
equation: aimc.idq_rs[1] = aimc.airGap.i_rs[1]
equation: aimc.idq_rs[2] = aimc.airGap.i_rs[1]
```

The right-hand index does not advance with the left. The machine hands
its base `extends PartialBasicInductionMachine(final idq_rs =
airGap.i_rs)`, and a value handed down an `extends` is spread over the
elements of what it binds. Spreading asks how long the value is, and
the shapes travelling with a modifier were collected under short names
only - a name with a dot in it was dropped outright. `airGap` is a
component standing _beside_ the `extends`, not a declaration of the
class, so `airGap.i_rs` was measured nowhere, came back whole, and was
bound to every element in turn. Two illnesses at once: a wrong
equation, `idq_rs[2] = i_rs[1]`, and one equation per pair where one
per element was owed - which is precisely the surplus counted.

A twelve-line model shows it whole and is now
`tests/small/a_modifier_naming_a_member_of_a_sibling_component.mo`.

The first fix was too broad and the corpus said so at once: measuring
the members of every component of every class cost **nine models of
flattening** (820 to 811) and took flattening from 1684s to 2133s, a
third again. A shape measured under constants that do not apply is
worse than no shape. Narrowed to the members a modifier actually
names - and only a bare `Ref`, only a plain member, only under a
scalar component - the pass is free: flat and ran lists identical
model for model, and `DoublePendulum` at 50.6s against 50.8s without.

The numbers are 820 / 376 / 722 / 371, unmoved. This is deliberately
recorded as a correctness fix and not a count: `IMC_DOL` went from a
surplus of 22 equations to 14, so the family moved two-thirds of the
way to its wall and stopped at another. What was removed is a wrong
equation, which by the rules of this project is the worst thing the
compiler can produce and worth removing on its own terms. The
remaining 14 are a different family - `fixed.flange.phi =
airGap.support.phi` and its neighbours, a connection matter rather
than a shape one - and that is where the next shift on these machines
begins.

## Shift 23: the mode-wise substitution, sieved and measured

### What was asked

Answer 22 in `QUESTION_FOR_FABLE.md` proposed variant 2: settle an `if`
_expression_ whose condition holds still into the branch that holds, on
the reasoning that a conditional left standing gives the algebraic layer
a slope that is zero down one arm, and a block torn on such a slope
divides by that zero. The wall it was aimed at is the models whose
residual was never a number.

The instruction was to sieve first, write second, and measure third.
All three were done, and the third says the substitution does not pay.

### 1. The sieve

The wall is 29 models, not the 39 the census suggested: the census
counts _kinds_ of refusal and `library check --refused` prints one line
per model, so several census rows share models. Measured with
`./target/release/oxidelica library check .msl --refused`, counting the
four kinds of run-time loop failure:

| kind                                | models |
| ----------------------------------- | ------ |
| residual ... is NaN before any step | 21     |
| algebraic loop did not converge     | 4      |
| singular Jacobian                   | 3      |
| algebraic loop diverged             | 1      |
| **total**                           | **29** |

Each was then run under `OXIDELICA_MODE_PROBE=1`, which prints every
`if` inside a torn block together with the names its condition reads and
whether any of them is an unknown of that same block. Classified by the
answer:

| family | models | meaning                                                        |
| ------ | ------ | -------------------------------------------------------------- |
| A      | 14     | every condition reads only parameters and discretes            |
| AB     | 8      | both kinds present in one model                                |
| B      | 4      | at least one condition reads a continuous unknown of the block |
| none   | 3      | no conditional in the block at all                             |

The prediction was that a third family would be found if there was one,
and there is: three models - `Resistor`, `DCSE_SinglePhase` and
`DC_CompareCharacteristics` - stand at the NaN wall with no `if` in the
guilty block. Their residual is not a number for some other reason
entirely, and no mode-wise anything will move them.

So the number the sieve gives, _before the code was written_, is: at
most 22 models (14 A plus 8 AB) could possibly be helped.

The pipe for every count above is
`./target/release/oxidelica library check .msl --refused` for the wall
and `--only <Class>` under the probe for the classification, model by
model over all 29.

A note on the sieve's first run, because it nearly went into this
report as fact: it wrapped the probe in `timeout 120`, which does not
exist on macOS, and every one of the 29 models came back with zero
conditions found. Twenty-nine honest-looking zeros in a third of a
second. This is the failure `AGENTS.md` already describes under "a zero
counts only where the same pipe can print something other than zero",
and it was caught only because 29 identical zeros arriving that fast was
implausible, not because the pipe complained.

### 2. The small models

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
honest small witness found was a model with a conditional _coefficient_
whose else-arm is `0/0`, which fails at `t = 0` on the base compiler
and at the flip with the substitution in place.

### 3. The change, and why it is parked

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
folded. A branch that becomes `0 * z` still _names_ `z`, the matcher
pairs the equation with it, and the division by zero happens exactly as
before.

Then the corpus, from one binary each time:

| gate                     | flatten | run | runnable flatten | runnable run |
| ------------------------ | ------- | --- | ---------------- | ------------ |
| baseline (s22)           | 820     | 386 | 722              | 381          |
| parameters and discretes | 820     | 364 | 722              | 359          |
| parameters only          | 820     | 386 | 722              | 381          |

The wide gate costs 22 models net: 26 lost against 4 won. The run-list
diff names them, and they are one family - `CharacteristicIdealDiodes`
and eighteen bridges of `Modelica.Electrical.PowerConverters`. The
cause is that a discrete looks like it belongs in the gate and does
not. A diode's `off` is discrete and does hold still _between_ events,
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

So variant 2 is parked rather than taken, and kept as a patch outside
the repository so that nobody has to build it again; what is committed is the probe, which is what made the
verdict measurable, and which cost the corpus nothing because it prints
only when its environment variable is set.

### What a later shift would need

The sieve says the models are there - 22 of them can see a settled
mode - and the wide gate proves the mechanism works, since it won four.
What it lacks is a way to tell a discrete the event iteration has
finished deciding from one it is still deciding. That is a question
about the event loop rather than about the algebraic layer, which is
the shape of a question for a consultation rather than for another
local attempt.

### The floors

Unmoved, and correctly so: 2671 / 0 / 820 / 386 / 722 + 381. The
committed change is an instrument and moves no number.

## Shift 24: the machine wall probed, and a definition that grows

The largest row of the run half's census is 51 models refused for
`cannot differentiate through algebraic variable`. Fifteen are
multibody and parked behind their own panel; twenty-five are machines,
in three shapes, and each was confirmed refused today with `--only`
against `.msl`:

| shape       | models | first witness          | variable                         |
| ----------- | ------ | ---------------------- | -------------------------------- |
| flange      | 10     | `IMC_DOL`              | `aimc.inertiaRotor.flange_b.phi` |
| air gap     | 11     | `SMPM_NoLoad`          | `smpm.airGap.spacePhasor_s.i_`   |
| transformer | 4      | `TransformerTestbench` | `transformer.l2sigma.plug_n.pin` |

The mechanism is not the one the multibody model shows. It is not the
skip of the equation under reduction: a probe printing every candidate
definition the fixpoint threw away, and the demoted states beside it,
names the cause outright. `aimc.inertiaRotor.phi` is already demoted at
the eighth reduction - it is in `dummies`, and its derivative is the
dummy that replaced it, one lookup for `differentiate`. The fixpoint
gathering definitions accepts one only when every name in it is a
parameter, a state or already accepted, and a demoted state is none of
the three. So `flange_b.phi = inertiaRotor.phi` was discarded and the
flange was called a variable nothing can differentiate.

Counting a demoted state as ground is four lines, and it works on the
models: `IMC_DOL` and `TransformerTestbench` pass the wall and stop at
the next one - `aimc.fixed.flange.phi = aimc.airGap.support.phi`
constrains no state - which is a kind removed. The air gap shape does
not move. The smallest witness is a machine and a load in seven lines,
now `tests/small/derivative_through_a_demoted_state.mo`.

It is parked, and for a reason no small model showed. Under the rule,
`Modelica.Electrical.Machines.Examples.ControlledDCDrives.CurrentControlledDCPM`
grows without bound and is killed; two corpus runs died at the same
420th model with the gate on, and the same binary without it completed
in the usual time at 820 / 386 / 722 / 381. So there is no corpus
number for the rule, and there will not be one until the growth is
understood: a definition reached through a dummy is a definition
reached through the equation that determined the dummy, and somewhere
that walk stops being finite. That is the next shift's question, and
it is a question about termination rather than about machines.

Two more models stand at the same census row and are not machines:
`ModelicaTest.Tables.CombiTable2Ds.Test33` and `CombiTable2Dv.Test33`,
both refused for `trapezoid1.T_start`. They do not move under the rule
either, and they are a different family - a source's start attribute,
not a connection.

## Where the growth came from, and what it was not

The question left above was a question about termination, and the
answer was measured rather than guessed. The instrument was the probe
already built, printing the reduction number and the expression under
it; the fork put to it was whether the reduction number climbs for
ever, which is a fixpoint demoting in a circle, or stands still while
memory grows, which is an explosion inside the walk.

Neither, as they were put. The reduction number stood still at twenty,
and the expression under it went 986, 1910, 14112, 90847, 4979469,
111942871 characters. A factor of twenty-two per reduction, killed for
memory rather than looping: the walk was finite all along and its
answers were not.

The shape in the dump named the cause. Every division carried
`Bin(Pow, Ref("dcpm.inertiaRotor.J"), Number(2.0))`, then
`Pow(Pow(J,2),2)`, then that squared again. The quotient rule is
written the general way and always emits `b^2`, so a division by a
parameter answers `(a'*J - a*0)/J^2`. The zero folds; the `J` against
`J^2` does not, because nothing in `simplify` cancels a name against
its own square. Harmless once - and index reduction differentiates its
own output, so the residue is squared once per reduction. `tau/J` is
the shape of every rotational model in the library.

The rule that replaces it says the identity instead: a denominator
whose derivative is zero divides the derivative and nothing else.
Narrower than a cancellation rule, true everywhere, and it needs no
arithmetic on `simplify`'s part.

Asking it of the _derivative_ rather than of the denominator is what
made it cover both doors, and the second door was found by measuring
after the first was shut. Guarded only for time, the model stopped
dying of memory and still grew - 9840, 57642, 2939832, 93577290 - and
the reason is that `solve_linear_for` differentiates by _variable_, not
by time, where a test on `does_not_move` is inert by construction. It
mints the candidate definitions the fixpoint hands back to the walk, so
it was squaring `L` and `J` on the way in. One identity, two entrances.

The fix ships on its own, without the parked rule. Its corpus numbers
are 820 / 386 / 722 / 381 - unchanged - and the run list diffs empty
against the previous shift's, name for name, which is the witness a
change of this kind owes. It is not a change that adds a definition and
so has no victims to map; it is a change that stops an answer growing.

What it bought under the parked rule is the difference between a
verdict and a corpse: `CurrentControlledDCPM` used to be killed, and
now reaches a refusal. It takes 338 seconds to do it, and a corpus run
with the gate on is still killed - on a different model, later in the
pass, past the 840th, which the previous shift never saw because the
first victim stopped the run at the 420th. So the barrier moved and did
not fall.

What remains is the inlining itself. A definition is expanded wherever
its name appears and shared nowhere, so the same `der(loadInertia.w)`
subtree is written out 125 times in one expression at reduction 18.
That is what still makes ninety million characters out of twenty
reductions, and it is a wall about common subexpressions rather than
about machines or about differentiation. The parked rule waits on it.

## A definition inlined is a definition copied

The wall the previous shift left standing was measured before it was
touched, and the two numbers it wanted came from one binary with the
inlining counted on one side and answered from a table on the other.

`CurrentControlledDCPM` meets 83 distinct definitions in a single index
reduction, and inlines them 5,200,549 times between them. The heaviest
are `der(loadInertia.phi)` and `loadTorque.w` at 927,888 apiece; four
of the machine's currents follow at some 476,000 each. The size of the
differentiated constraint over twenty reductions is 1,370, 9,744,
57,546, 2,939,736, 93,577,194, 5,541,329,537 - the last is five
gigabytes, and it is where the pass was killed.

The second measurement is the one that decided the fix. A table of
answers - the derivative of each name worked out once and remembered
for the rest of the call - left the sizes **identical to the digit**,
1,370 through 5,541,329,537. Nothing about a repeat is dear except the
copy: a remembered tree is cloned into every occurrence exactly as a
freshly worked one is. So memoisation was the wrong door, and this is
the general shape worth keeping: a table trims repeated _work_, and
what this cost was repeated _shape_.

What the shape wants is a name. The derivative of a definition is now
minted as `der(x)` and defined once, with the occurrences left as
references - the move Pantelides already makes for a demoted state,
made for an algebraic one. The name is claimed before its body is
worked out, so a definition reaching itself through another finds a
reference rather than recurring; and it is minted only where nothing is
held still, because inside an implicit derivative the chain of held
names changes what the answer is, and one name cannot carry two.

The effect on the model that prompted it: the constraint no longer
grows at all - 90, 34, 42, 92, 75, 59, 75, 81, 76, 81, 48, 69, 91
characters over thirteen reductions, against thousands and then
millions - and the model that took 338 seconds to reach a refusal now
takes 0.33. The refusal behind it is `cannot differentiate through
algebraic variable`, which is a wall of its own and not this one. With
the rule on, a corpus run under the parked demotion gate reaches the
end for the first time instead of being killed for memory.

And the rule is parked all the same, because the list of victims says
what no total could. Measured alone, without the demotion gate, it
costs four models and wins two: `DCPM_Start`, `DCPM_withLosses`,
`Translational.Examples.Brake` and `TestFrictionPosition` against
`FilterWithDifferentiation` and `SinglePhaseInductance`. Run with the
gate as well, the two lists are identical name for name, which says the
four are this rule's doing and not the gate's.

The cause is worth stating, because it is the same fact seen from the
other side. Inlining a definition is also what _flattens an algebraic
loop_: written out, `brake.a` and the force beside it collapse into
something the tearing can plan, and kept as names they stay a loop it
cannot. So the copying this fix removes was paying for itself
somewhere, and the choice between the two compilers is not one a
performance fix gets to make quietly. It ships behind
`OXIDELICA_SHARED_DERIVATIVES=1`, with the numbers above as what it
costs and what it buys.

## What the four victims of the shared derivative actually are

The rule was parked with a list of four names and no account of why
each was on it. Probed one at a time, they are two causes and not one,
and neither is the tearing being unlucky.

`DCPM_Start` refuses with `cannot differentiate through algebraic
variable dcpm.inertiaRotor.a`, and the probe over the settling
fixpoint says why the name has no definition to reach through. Among
the candidates for it are `a := der(w)` and, minted by an earlier
reduction, `der(w) := a`. Each is a definition of the other, so the
fixpoint accepts neither and the walk meets `a` with nothing under it.
Grounding minted names as axioms - counting them settled because they
carry their own equation - was tried and overflows the stack in a
second: the cycle is real, and hiding the acyclicity test only lets
the walk run round it. What the family wants is the pair recognised as
one fact, `a` and `der(w)` being the same quantity under two names,
which is a question about how a minted name is related to the state it
came from rather than about the order of the fixpoint.

`Translational.Examples.Brake` refuses in the solver, and the block it
refuses in is the finding. With the rule off the loop holds twenty
unknowns and tears `brake1.a - mass2.a`; with it on, the minted
`der(der(brake1.s_a))` and its neighbours join, the loop holds
twenty-six, and both brakes are inside one block instead of one each.
The refusal now names where the fault entered - `brake1.sa` is NaN
before any Newton step - and `brake1.sa` is divided by a coefficient
that is `unitForce` when locked and zero in every other branch. That
divisor was always zero at t = 0; what changed is that the second
brake is now in the block whose starting values are evaluated, so a
division that used to sit outside the loop is now inside it. The wall
is the mode-dependent slope already counted as family A above, met by
one more model because the block grew.

So neither victim is evidence that keeping a name is wrong. One is a
cycle between a minted derivative and the algebraic it defines; the
other is an existing wall reached by a bigger block. Both are worth
their own work, and until it is done the gate stays parked.
