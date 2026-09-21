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

## The second wall: differentiating through an algebraic variable

At 49b13e5 the census of refusals puts 51 models on `cannot
differentiate through algebraic variable`, second only to the 137
unbalanced, and unlike that one it is not a mixed bag. Two shapes of
equation account for nearly all of it. One is an alias, `x = y`:
`aimc.inertiaRotor.flange_b.phi = aimc.flange.phi` in the induction
machines, `frame.R.T[i,j] = ...` in MultiBody, DoublePendulum among
them. The other is the pair a connection writes, `x + y = 0`:
`spacePhasor_s.i_[k] + lssigma.spacePhasor_b.i_[k] = 0` in the
synchronous machines, and the same shape in the transformers and in
FluxTubes.

The two shapes are not one fault. For the alias, the probe on IMC_DOL's
eighth reduction shows `aimc.inertiaRotor.phi` sitting in the dummy
table - a name whose derivative the walk already knows how to take -
while the three candidates around it, each an alias of the other, all
fail to settle. The fixpoint counts as ground what it has accepted and
what the implicit rule grounds, and does not count the dummies,
although the implicit rule itself filters them out of its unsettled
list. So the flange stands one hop from a known derivative and the hop
is not taken.

Counting dummies as ground was tried behind a switch and measured from
one binary. It takes IMC_DOL off this wall and puts it on the next one
along, `constrains no state`; it leaves SMEE_Generator and
DoublePendulum exactly where they were; and it makes the corpus run out
of memory at seven threads, at four and at two, while neither
DoublePendulum nor IMC_DOL grows by a byte when run alone. Some model
in the corpus explodes when that condition is lifted and the ordinary
pass cannot say which. The change was reverted.

The pair is a different animal, and the probe says so plainly. On
SMEE's fourth reduction the only candidates for the current are
`airGap.i_ss[2] := spacePhasor_s.i_[2]` and its inverse, which is a
two-name cycle by construction, and the connection equation itself is
deliberately withheld from the implicit rule because taking it would
travel the circuit for ever instead of reaching the flux that moves.
There is no ground to reach. The current is determined by the flux, and
the flux equation names two unsettled names, so the rule's condition of
a single unsettled name rejects that too.

That is the same door as DCPM_Start: the current wants minting as an
unknown in its own right with its equation handed to the matching,
which is architecture rather than a local fix. The question has gone to
the consulting model with both shapes in it, because taking the alias
alone leads only to the wall next door.

### Naming the model that eats the machine

The measurement above had a hole in it: the corpus died of memory at
every thread count and said nothing about which model was responsible,
because the death was the operating system's and not the compiler's.
A ceiling on the size of a differentiated constraint closes it. Index
reduction counts the nodes of each folded derivative and refuses past
two million, naming the equation, the reduction and the size reached,
which is a sentence rather than a corpse.

Run against the same probe, the corpus that used to die now finishes
and names two models: `CurrentControlledDCPM`, whose constraint reaches
five and three quarter million terms at the nineteenth reduction, and
`SpeedControlledDCPM`, three and a half million at the eighteenth.
Both are the constraint copying its own shape down the chain of
reductions rather than anything peculiar to their physics, and both are
in the family the shift was already mapping.

The ceiling is worth having on its own account. Two million nodes is
far above what any healthy model of the library reaches, so nothing
that runs today notices it, and any future change to the grounding
rules is measurable instead of fatal.

### Minting a derivative the matching can supply

The door the last section named as architecture turned out to open.
Substitution cannot reach the derivative of a current in a circuit: a
Kirchhoff node rewrites into its neighbour and round the loop for
ever, which is why the walk refused. The matching does not rewrite, it
assigns - every unknown has exactly one equation determining it - so
the equation the walk could not find by substituting is one the caller
already holds. The walk now takes a name of its own for such a
derivative and records the debt; `reduce_index` finishes the matching
over the remaining equations, differentiates the one assigned to the
name, and puts the pair into the system. A name matched by nobody even
then is one nothing determines, which is the old refusal in its old
words.

The reach that chooses the victim was widened to the equations the
matching supplied as well as the implicit ones. Both are equations
that determine a name without defining it, and a reach that stopped at
a minted name would report a constraint that does pin a state as
pinning none.

Measured from one binary with the change behind a switch: 820 flatten
either way, 386 run without it and 389 with. The three are
`Machines.Examples.Transformers.TransformerTestbench` and both
`FluxTubes.Examples.MovingCoilActuator` models. The run list has no
victim in it - the diff is acquisitions only - and the unbalanced
family, summed across the wordings that split it, stands at 137 models
before and 137 after, which is what says the supply ledger is exact:
one name in, one unknown and one equation out.

The two models that prompted the work are not among the three, and
that is the expected shape rather than a disappointment. `SMEE_Generator`
passes the wall and stops at the next one, a constraint on
`der(smee.inertiaRotor.flange_b.phi)` that pins no state; `IMC_DOL`
passes it and stops in an algebraic loop that starts from values that
are not numbers. A wall taken is not a model bought, and the two are
recorded here as walls taken.

The witness is the corpus census rather than a small model. Nine
attempts at shrinking one failed: the path switches on only where
substitution walks a Kirchhoff node round a circuit, and a circuit
small enough to write in a test settles by substitution before the
path is reached.

## The slowest model in the library, and why it was slow

`DoublePendulum` has been the slowest model in the corpus for as long
as anyone has measured it, and the number had been quoted as
forty-five seconds. Measured over three commits from one machine it
was 59 seconds at the commit before the ceiling guard, 57 seconds with
it, and 117 once the rule for the derivative of a name the matching
determines landed. That rule is on by default, and it had doubled the
model. The library job's trend followed: 64 minutes, then 73, then
past the 90 minute ceiling, where the run was cancelled rather than
failed. A cancelled run leaves the same blank space as a green one, so
the floors of the run that was cancelled were checked by nobody.

The profile put 2264 of 7300 samples inside `reduce_index` in
`solve_linear_for`, which is asked for every equation and every name
in it, at every reduction. It differentiates the equation and folds
the result three times, and the answer is the same every time it is
asked: inside one `reduce_index` the list of equations is only ever
appended to, so the pair at an index is the pair that was there
before. Held still by its index, the key is the index and the name -
which is the bracket the note about tables asks for, found by asking
what makes two askings different rather than by reaching for a cache
first.

The corpus run half fell from 2062 seconds of processor time to 633,
`DoublePendulum` from 117 seconds to 27, and the list of models that
run is identical in both directions - 389 before, 389 after, no model
in the difference either way.

The measurement also turned up a race that had been sitting in the
tests. The test for the constraint ceiling lowered it through the
environment, which belongs to the whole test binary rather than to one
test, so any test compiling beside it saw a ceiling of one node and
refused a model it should have run. A mutex around the setting cannot
help, since the readers are the other tests and they hold no lock. It
went unseen because the model it struck took two minutes to reach the
read; it surfaced the moment that model became fast. The ceiling is
now lowered for one thread, as the shared-derivative switch already
was.

## What the shared-derivative rule costs, model by model

The rule that gives the derivative of a definition a name of its own
is parked behind `OXIDELICA_SHARED_DERIVATIVES`, and the reason is
that it buys two models and sells five. The five were probed one at a
time against the root of the corpus, and the wall each of them stops
at is worth naming, because they are not one family:

- `DCMachines.DCPM_Start` and `DCMachines.DCPM_withLosses` stop at
  `der(der(dcpm.inertiaRotor.flange_b.phi)) - der(der(dcpm.flange.phi))
= 0`, a constraint that pins no state. The two derivatives are a
  rigid connection differentiated twice, and with the definition
  behind a name the reduction can no longer see through it to the
  state either side stands for.
- `Transformers.TransformerTestbench` stops at
  `transformer.starpoint2.i = 0`, the same kind of thing one storey
  lower: a current pinned to zero, constraining no state.
- `Translational.Examples.Brake` stops in an algebraic loop over
  `brake.a` and `brake1.flange_a.f` that the tearing cannot plan.
  Inlining a definition is also what flattens a loop, so keeping the
  name is what builds this one.
- `ModelicaTest.Translational.TestFrictionPosition` stops with a loop
  whose residual is infinite before the first Newton step, every value
  in the block starting at minus infinity.

`TransformerTestbench` is worth reading twice: the minting change of
the previous shift bought it, and this rule sells it. Two rules
interfering over one model, which is the clearest possible statement
that these are two compilers rather than one compiler and a fix.

Two notes on the probing itself. The rotational namesake of the
friction model is not a victim - `ModelicaTest.Rotational.TestFrictionPosition`
runs with the rule on, and only the translational one falls, so a
victim named by its tail is as much of a guess here as anywhere else.
And the probe has to be pointed at the root of the corpus: `--only`
against a subtree answers `unknown base class`, which reads exactly
like a refusal caused by the change under test and is not one.

## The residual-is-NaN row: one wall in four storeys, not forty-one families

The top of the run half's register is `residual N of algebraic loop` at
forty-one models, with `algebraic loop did not converge` (6) and
`singular Jacobian` (3) beside it. Probed, the three rows are not one
family and the forty-one are not forty-one causes. What separates them
is a phrase already in the message: every one of the forty-one says
`at t = 0, before any Newton step`. Nothing has diverged, because
nothing has stepped. The other two rows are the honest ones - a loop
that was evaluated and would not converge, or would not determine its
answer - and they belong to the rectifiers, where a bridge of ideal
diodes really is hard. So the register's top row is a wall of its own,
and the neighbours are not it.

Behind the wall is a chain, walked here on `Modelica.Electrical.Analog.Examples.Resistor`
until a twelve-line model showed it whole. The block's inner unknowns
are solved symbolically and evaluated in order, so an equation linear
in its unknown becomes a division, and each link is a divisor that is
zero where the model begins:

1. A torn variable with no `start` of its own gets zero, and
   `v = R_actual*i` solved for `R_actual` divides by it. This is the
   link the census sees, and it is a guess rather than a fault: the
   same model with `i(start = 1)` runs and gives the right numbers.
2. A slope that is a parameter valued zero. `R_actual = R*(1 + alpha*
(T_heatPort - T_ref))` with `alpha = 0` does not mention
   `T_heatPort` at all, and solving it for that temperature divides by
   nothing. This link is taken by the change of this shift.
3. A slope written as a literal zero, the same fault a fold earlier.
4. A source that is zero at the start. `SineVoltage(f = 1)` is exactly
   zero at `t = 0`, and the current it drives is the divisor of the
   link above; started a quarter period along, the same model runs.

The links are one mechanism seen at four depths, which is why removing
any single one moves no count: the model dies one step later in the
same family. The map is written down here so the next shift does not
re-derive it, and so the two halves can be told apart - links 2 and 3
are a wrong answer where a refusal was owed, and are worth taking on
their own account whatever they cost in models; links 1 and 4 are a
guess about where to start Newton, and are worth nothing unless the
guess can be made without inventing a number.

That last distinction was paid for in this shift. Retrying a start of
zero a little off the origin takes links 1 and 4 together, and it was
built, measured and thrown away: `1 / x = 0` has no solution, and from
a nudged start Newton walks off happily towards one and the model
compiles. A refusal became a wrong answer, which is the worst thing
this compiler can do, and the test that says so was already written.
So the guess is not available as a global rule. What might be is a
start taken from the model rather than invented - the value a
connected set's other member states, or a medium's own default - and
that is the shape the next attempt should take.

What shipped is the rule that needs no guess: an equation is not
divided through by a coefficient that is zero. It is true everywhere
rather than almost everywhere, and where it fires the refusal changes
from `residual is inf` - which names the solver, the one place nothing
is wrong - to `underdetermined algebraic loop`, which names the thing
that is actually undetermined.

Measured, that rule is worth two models and no losses: 389 run to 391,
384 runnable to 386, the run lists differing by `PolyphaseInductance`
and `SinglePhaseInductance` of `Magnetic.FundamentalWave` and by
nothing else. The register moved by three rather than two - the
residual row went 41 to 38 - and the third model is the reason to read
both instruments: it did not run, it moved to `structurally singular
model: the equation determining X does not depend on it`, which is the
same fact stated where it belongs. A row emptying by three while the
run count rises by two is exactly the shape the charter describes, and
here both halves were visible in one pair of censuses.

### The shares behind that row, and what a start can be read from

The map above said the row stands on two guesses and two faults, but
nobody had counted how the models divide between them. Probed one by
one - forty-one models of the census, each asked with `library check
--only` and `why` about the variables its own message names - the
division is:

| where the model stands                       | models |
| -------------------------------------------- | ------ |
| torn variables, none of them given a `start` | 32     |
| at least one torn variable given a `start`   | 9      |

The nine are the interesting half, because a start that exists and a
refusal that says the block began from nowhere cannot both be true.
They are not: `HeatingMOSInverter` hands Newton `288.15` for both its
temperatures, and the block still fails, because what is infinite is
an _inner_ assignment evaluated after the torn values are placed.
Which settles the question the map left open - link 1 is not one thing.
A start present and carried is a different case from a start absent,
and only the second is about where Newton begins.

For the second the charter's rule stands: a number invented is a wrong
answer waiting, and last shift's nudge proved it. But a number _read_
is not invented. `T_port = flowPort.h/cp` with `T_port(start = 288.15)`
says what `flowPort.h` starts from exactly as plainly as a `start` on
`flowPort.h` would, and the silent zero taken instead is not neutral -
in this family it means absolute zero, and the next equation divides
by it. So a torn variable with no start of its own now takes one from
an equation that names it and nothing else the model has not already
valued, and only where it enters linearly, which is the one case with
a single answer. Everything else keeps its zero.

Measured over the corpus from one binary, with the rule behind an
environment switch: 820 flatten and 391 run both ways, and the two run
lists are identical name for name. The rule costs nothing and wins
nothing today. It shipped anyway, because it is a _reading_ of the
model where there was a guess, the case it fixes is real and has a
test that fails without it, and the walls it does not reach are the
ones the shares above locate: the FluidHeatFlow family gets its start
read and then dies further in, at inner assignments whose divisors are
still zero. That is the chain's next link, not this one.

Two things were learned about cost on the way. The first shape of the
rule scanned every equation for every unknown, which took one model
from seconds to twenty-five minutes - the quadratic the charter warns
about, met head on. Narrowed to one pass over the equations, and then
to the torn variables alone - the only ones Newton ever reads a start
for, since the rest are assigned outright before the residual is
formed - the pass time is back to nine minutes, under the eleven it
has always taken. The second: a binary rebuilt between the two halves
of a comparison makes the comparison worthless, which was caught here
only because the numbers were taken again afterwards from one build.

### The link behind a start that was read: an honest zero, from the wrong end of a product

The shares above left the nine open: the start is present and carried,
and the block still cannot be evaluated. Probed on
`Electrical.Analog.Examples.HeatingMOSInverter` - the torn values and
every inner assignment printed as the block first forms its residual -
the answer is the map's link 4 and not a fault of the reading.

The two torn temperatures arrive as `288.15`, exactly as the reading
put them there, and thirty-eight of the block's forty inner
assignments produce ordinary numbers. The order they are evaluated in
is a correct topological one; that was the first suspicion and it is
wrong. What the printed order does show is which way each equation was
solved:

```text
H_NMOS.LossPower <- ["H_NMOS.heatPort.Q_flow", "H_NMOS.LossPower"]
H_NMOS.D.i       <- ["H_NMOS.LossPower", "H_NMOS.D.i"]
H_NMOS.id        <- ["H_NMOS.D.i", "H_NMOS.id"]
H_NMOS.v         <- ["H_NMOS.v", "H_NMOS.ugst", "H_NMOS.id"]
H_NMOS.beta_t    <- ["H_NMOS.v", "H_NMOS.beta_t"]
```

Read downwards this is the transistor's own chain run backwards. The
matching gave `LossPower = D.i*(D.v - S.v)` to `D.i` rather than to
`LossPower`, so the assignment is a division by `D.v - S.v`, and every
link below it inherits the infinity: `id` from `D.i`, `v` from `id`,
and `beta_t` from `v` - which is how a quantity that cannot be
negative for any non-negative temperature is printed as `-inf`. The
device's forward direction never runs. `H_PMOS`, matched the other way
round, evaluates the same five equations in the writing order and gets
finite numbers throughout.

The divisor is honestly zero, and that is the finding. `S` sits on the
ground through `Capacitor1.n.v = G.p.v = 0`, and `Capacitor1` starts
from `v(start = 0, fixed = true)`, so `D.v - S.v` is zero at `t = 0`
because the model says the inverter starts with its output node
discharged and no voltage across the device. Nothing is missing and
nothing was guessed: this is a real zero of a real product, met from
the end of the product that has to divide by it. So it is case (ii) -
link 4 of the map, a source that is zero at the start - and no reading
of starts can reach it, because there is no start to read.

What stands behind it is therefore not a start at all but the choice
of direction: a matching free to take `LossPower = D.i*(D.v - S.v)`
for `LossPower`, whose evaluation is a multiplication and cannot
divide by anything, took it for `D.i` instead. Whether tearing can be
made to prefer the direction that multiplies over the direction that
divides is a question about the matching, and it is not decided here.

### The remainder of the wall, probed: one mechanism, four costumes

The wall stands at thirty-eight models on the corpus as measured this
shift (the forty-one of the earlier census less the three the divide-by-zero
refusal took). Four of them were probed to answer the narrow question
the shares left: is what stands behind them a start that could be read
from somewhere, or something else entirely.

| model                                  | torn variable probed  | what the assignment divides by      | is the divisor honestly zero                                 |
| -------------------------------------- | --------------------- | ----------------------------------- | ------------------------------------------------------------ |
| `Rotational.Examples.CoupledClutches`  | `clutch3.tau`         | `unitTorque` in an erased branch    | no - a live branch, `sa` is `inf` from above                 |
| `FluidHeatFlow.Examples.SimpleCooling` | `pump.flowPort_b.h`   | `medium.cp` (finite), then `m_flow` | no - `h` arrives `inf` from a neighbour                      |
| `Machines.InductionMachines.IMC_DOL`   | `aimc.airGap.i_sr[2]` | `RotationMatrix[1,2] = -sin(gamma)` | yes - `gamma = 0` at rest                                    |
| `Media.Examples.ReferenceAir.DryAir1`  | `volume.medium.T`     | a media inversion                   | not reached - `T` has both a `start` and an initial equation |

Two things follow, and they are what point four was for.

The first is that the extension the last shift left on the table - a
fixpoint that puts a read start back into `stated` so chains of length
two propagate - would not reach these. In three of the four the torn
variable's neighbours are not unvalued: they are valued `inf` or `NaN`
already, having come through an assignment that divided. There is no
start missing to read; there is a division that should not have been
the direction chosen.

The second is that `IMC_DOL` is `HeatingMOSInverter` again in another
costume. `i_ss[1] = R[1,1]*i_sr[1] + R[1,2]*i_sr[2]` was matched to
`i_sr[2]`, so evaluating it divides by `R[1,2] = -sin(gamma)`, and
`gamma` is zero because the machine starts at rest. The equation read
the other way round is a multiplication that cannot fail. The rotor
angle being zero at `t = 0` is not a defect to be repaired by reading
a start from somewhere: it is what standing still means.

So the question that both halves of the wall now point at is the same
one, and it is not about starts at all. It is whether the tearing may
prefer, among the equations a variable could be solved from, one whose
solution multiplies over one whose solution divides - and, where it
must divide, whether the coefficient can be checked against zero at
the values the block starts from rather than after the infinity has
propagated. That is a question about the matching and about symbolic
division, and by the standing arrangement it is not decided here.

## The matching may prefer to multiply, and what that uncovered

The question the last shift left open - whether the matching may
prefer, among the unknowns an equation could be given, one whose
solution multiplies over one whose solution divides - is now measured
rather than argued.

The smallest model with the shape is twelve lines and refuses in one
second:

```modelica
model Inv
  Real c; Real a; Real b;
equation
  c = time;
  a = b * c;
  b = 1 - a;
end Inv;
```

`a = b * c` mentions both `a` and `b`, and the matching was
indifferent between them: it walked `eq_vars` in the order the names
were collected and took the first that a path was free to. Given to
`b` the assignment is `-a / -c`, and `c` is zero at `t = 0`, so the
block's first residual is a NaN before Newton has taken a step. Given
to `a` it is a multiplication that cannot fail.

The preference is a rank over each equation's unknowns, cheapest
first: nought where the unknown stands alone on one side, one where
the slope names no other unknown of the block, two where solving
divides by something that moves under Newton. It is only a
preference - an augmenting path may still overrule it - which is why
the rank was measured rather than believed.

Measured from one binary with `OXIDELICA_NO_MATCH_ORDER`, the corpus
goes from 820 flattened and 390 run to 820 and 392, and the runnable
pair from 722/385 to 722/387. Two models won: the diff of the run
lists is `Modelica.Electrical.Analog.Examples.HeatingRectifier` and
`Modelica.Thermal.FluidHeatFlow.Examples.SimpleCooling`.

That measurement said "no victims", and it was wrong. The switch
brackets the matching's rank and not the regularity check that came
with it, so the diff of off against on could not see a victim of the
check by construction; the diff that could is the old binary's run
list against the new one, and it names
`Modelica.Magnetic.FluxTubes.Examples.BasicExamples.SaturatedInductor`.
It ran at `d87f8d5` and runs at neither the base of this change nor
its head. The shift's true arithmetic against the previous binary is
two won and one lost, which is the +1 the floors recorded and the
reason the base measured 390 against a floor of 391 - a discrepancy
that was explained away at the time by a difference between the desk
and the builder that does not exist.

The two models the last shift named are not among them, and that is
worth saying plainly rather than letting the count imply otherwise.
`HeatingMOSInverter` still refuses, and the probe says why: its
`-inf` comes from `beta_t = Beta * (T_heatPort/Tnom)^(-1.5)` with
`T_heatPort` at zero, which is a start missing rather than a direction
chosen. `IMC_DOL` still refuses too, its NaN entering through
`RotationMatrix[1,1]`. So the wall named in the last census was two
walls wearing one wording, and only one of them was the matching's.

### What the change uncovered, which is the larger finding

The rank turned a test red at once, and the red test was worth more
than the change: `x = y + 1` beside `y = x - 1` is the same equation
twice, and with the equations matched the other way round the
compiler ran it and printed an answer that is an artefact of the
initial guess.

The regularity check had been asking the wrong question. It builds
the block's Jacobian by finite differences and hands it to
`solve_linear`, which declines only when a pivot falls below 1e-14.
A column that cancels exactly in exact arithmetic does not come back
as zero from a finite difference: it comes back as noise of order
1e-8, nine orders above that floor, and reads as a live coefficient.
Which of the two readings of a degenerate loop was reached decided
whether the compiler refused it or guessed at it, and that is the
worst thing this compiler can do.

So the check now asks for the smallest pivot and judges it against
the Jacobian's own scale rather than against the floating-point
format. That is a question about the block - is there a direction the
residual barely moves along, compared with the rest of the block -
where the old one was a question about `f64`.

### The victim, and what the scale test costs where the units are large

The one model the regularity check cost was probed rather than
assumed, because the two possible stories - a genuinely degenerate
block that was previously guessed at, and a regular block newly
refused - call for opposite work. Printing the block's Jacobian at
the point of judgment settles it in one run:

```text
pivot 1e0  scale 3.3116960540533507e8  threshold 3.3116960540533505e1  n 4
block ["source.p.i", "r_mLeak.port_p.Phi", "r_mFe.B", "r_mFe.Phi"]
  row [1.0, 2000.0000000000002, 0.0, 0.0]
  row [0.0, 1.0, 0.0, 1.0]
  row [0.0, 0.0, 1.0, -1599.9999999999995]
  row [0.0, 1200000.0, 0.0, -331169605.40533507]
```

That matrix is regular, and not marginally: eliminated, it has no
small pivot in any absolute sense, and its determinant is far from
zero. What sinks it is the comparison. One row carries a magnetic
reluctance of order 1e8, which sets the whole matrix's scale, and the
threshold `1e-7 * scale` then stands at 33 - so a pivot of exactly
1.0, a coefficient of one in a row written in amperes, is judged
"barely moving" against a row written in reciprocal henries.

The scale test is right about what it was built for and wrong here
for a nameable reason: a single scalar taken over the whole matrix
compares coefficients of different physical dimensions. Rows of a
Modelica block have no common unit, and in a magnetic model the
spread between them is routinely eight orders. The block is not
ill-conditioned in the sense the check means; it is heterogeneous in
units, which every electro-magnetic model is.

The obvious repair - lower the constant - is not taken, because it
trades one arbitrary threshold for another and the test that prompted
the check would begin to slip through again at some spread of units.
What the numbers suggest instead is a per-row measure: judge each
pivot against the scale of the row it came from, so that a
coefficient of one in a row whose largest entry is two thousand is
seen as live, while a coefficient of 1e-8 in a row of order one is
still seen as noise. That is a change to how degeneracy is decided
for every block in the corpus, so it is written down with its numbers
and left for a decision rather than made on the strength of one
model.

## The mixing rule, and an equation that says nothing on one branch

The census taken at `f007b6c` puts `residual N of algebraic loop` at
the top of the run half with 35 models, and the first thing to say
about that row is that it is not one family. Sorted by which of the
block's own values went first, the largest single group is
FluidHeatFlow - ten models whose block starts with every enthalpy in
it a NaN - and behind it sit the machine models entering through
`RotationMatrix[1,1]`, the friction models entering through `sa`, and
several singletons.

The FluidHeatFlow group was probed to its mechanism. Printing the
first inner assignment that comes out not a number, for `OneMass`:

```text
first non-finite inner: pump.h = NaN
  code: Bin(Div,
    Neg(Bin(Sub, Slot(163), If(Rel(Ge, Slot(123), Const(0.0)),
                               Bin(Mul, Slot(185), Slot(123)),
                               Const(0.0)))),
    Neg(If(Rel(Ge, Slot(123), Const(0.0)), Const(0.0), Slot(123))))
```

The equation is the mixing rule of `BaseClasses.TwoPort`,
`flowPort_a.H_flow = semiLinear(flowPort_a.m_flow, flowPort_a.h, h)`,
which flattens to `if m_flow >= 0 then h_a * m_flow else h * m_flow`.
The matching gave that equation to `h`, and on the branch the model
starts in - the flow is positive - the right-hand side does not
mention `h` at all. Solved for it, the assignment divides by an exact
zero, and every enthalpy downstream of it is a NaN before Newton has
taken a step.

The smallest model with the shape is eleven lines, and what it does
is worse than refusing:

```modelica
model FHF2
  parameter Real cp = 1;
  Real h(start = cp * 293.15); Real T; Real m_flow; Real h_a; Real H;
equation
  m_flow = 1;
  h_a = cp * 300;
  h = cp * T;
  H = semiLinear(m_flow, h_a, h);
  H = m_flow * h_a;
end FHF2;
```

It reports a hundred successful steps and prints `h = NaN, T = NaN` as
though they were answers. That is the guessing this project refuses
elsewhere, arriving by a different road: no refusal is raised because
the block is explicit rather than torn, so nothing looks at the value.

### The rank that was measured and taken out again

The obvious repair extends the solve-cost rank of the previous shift
with a fourth level: a slope which is an identical zero down some
branch of its own `if` is the dearest reading of all, because in that
regime the equation does not determine the unknown for any values
whatever. Written, it needs to see through the negation a residual's
difference introduces and through a factor lifted out around the
switch, and then it fires - `h` is ranked 3 where it had been 2.

Measured on the family from one binary behind `OXIDELICA_NO_BRANCH_COST`,
it is a loss. Not one of the ten FluidHeatFlow models runs with it on:
the block is rematched, and the refusal moves from residual 0 to
residual 1 or 2 of the same loop. And `SimpleCooling`, which runs
without it, refuses with it. So the change is a victim with nothing
bought, and it was taken out rather than carried on the strength of
the mechanism being right.

What the measurement says about the mechanism is narrower than it
first appeared: the mixing rule's division by zero is real and is
where the NaN enters, but the matching cannot avoid it by preference
alone. Every enthalpy in the block sits in an equation of the same
shape, so ranking one of them dearer only moves which one is given
the vanishing branch. That points at tearing rather than at matching -
choosing the torn set so that no explicit assignment is a division by
a branch-dependent zero - which is a different layer and a larger
change than a rank.

## The rank that cost four run halves

The library job reached its ninety minute ceiling a second time and
was cancelled: 48, 42 and 45 minutes over three commits, then a run
killed at 90m25s with nothing to report. The suspect was the last
change to the compiler rather than the runner, and the switch the
change already carried made the measurement cheap. From one binary
over the whole library:

```text
rank on   flattening 1780s / 1043 models   running 3153s / 820 (3845ms each)
rank off  flattening 1645s / 1043 models   running  591s / 820 ( 721ms each)
```

The rank alone costs more than four times what the entire run half
cost before it existed. Nothing else in the pass moved: the flatten
halves differ by the noise of a desk, and the difference is confined
to the layer that changed - which is the stopping rule this project
already wrote down for a slow path.

The cause is the one the morning's fix had already named, met again
one function further along. `solve_cost` answered two questions in one
breath: the shape of the equation in the name, which is a
differentiation and a fold, and whether the slope mentions another
unknown _of this block_, which is a membership test. Only the second
moves between reductions, as states are demoted and the unknown set
shrinks. Answered together, the first was paid again at every
reduction for an answer that could not have changed.

Split into `solve_shape` - remembered by equation index and name,
under the same bracket `solved_for` uses - and a `solve_cost` that
reads the remembered shape against the current unknowns, the pass
goes from 12m31 to 7m43 and the run half from 3153 seconds to 749.
The run lists before and after are identical, 392 models each, so
this is the same compiler with the repetition taken out.

Which makes three instances of one method, and it is worth stating as
a habit rather than as three anecdotes: when a path is slow, do not
ask how to remember its answer. Ask which parts of the world the
answer depends on, and then which of those parts the surrounding
bracket already holds still. Both times here the answer split cleanly
into a dear half that depended on nothing that moved and a cheap half
that depended on everything that did.

## The census's largest row, and the chain behind it

`unknown function world.gravityAcceleration` is 16 models, the largest
single-cause row of the run half's census once the rows that only
share wording are added together. The mechanism took one second to
find rather than a corpus run, because the twelve-line model with the
same shape refuses the same way:

```modelica
model M
  model W
    function accel input Real x; output Real y;
    algorithm y := 2 * x; end accel;
  end W;
  W w; Real a;
equation
  a = w.accel(2.0);
end M;
```

`w` is a component, not a package. Every reading of a dotted name in
`lookup_at` - the leading dot, the named imports, the walk out of the
enclosing packages, the aliases, the wildcards - looks for a _class_
called `w`, and there is none. The name fell through unresolved and
reached the run as a call nothing could place. Modelica reaches class
members through an instance, and nothing in the compiler did.

The link itself is small: after every other reading has failed, look
for the head among the components of the class the name was written
in and of its bases, and ask the component's declared class for the
member. A test is red without it and green with it.

Measured from one binary, it is a loss of three and a gain of none:
820 flattened without it, 817 with, the run half 392 either way. The
three are MultiBody's constraint examples, and they do not fail for
the lookup - they fail one step further along, at
`standardGravityAcceleration is missing its argument gravityType`.
Reaching the function is precisely what lets them reach the next
wall.

That is the chain shape this repository already has a rule for, and
the rule says not to take it apart a link at a time. Two more links
are mapped, both reproduced small:

1. **The lookup.** Taken, behind `OXIDELICA_COMPONENT_MEMBER` and a
   thread-local guard for the test. Off by default: the numbers above
   are why.
2. **A short definition's modifiers, filled in from a constant.**
   `function accel = Scaled(c = 5);` inside a component works today -
   `a = w.accel(2.0)` gives 10.
3. **A short definition's modifiers, naming the holder's own
   parameters.** `function accel = Scaled(c = k);` where `k` is a
   parameter of the component gives `unknown variable k`. This is the
   link the MultiBody family actually needs: `gravityAcceleration =
standardGravityAcceleration(gravityType = gravityType, g = g*...)`
   names three parameters of `world`. The modifiers are remembered by
   `remember_filled_inputs` under the resolved _class_ name, a table
   with nowhere to put an instance path, so a value naming the
   holder's parameter arrives at the call without the prefix that
   would make it a name of the flat model.

Link 3 is the architectural one and is where the next shift starts.
The table is keyed by class, and what it needs to carry is per
instance; that is the same invariant this repository states as
"anything that survives flattening carries the flat model's names",
seen from the side of a table that cannot carry them.

### The chain walked to its end, and what it was worth

The five links, taken as one series after the map was walked rather
than one at a time:

1. **The lookup**, `world.gravityAcceleration` read through the
   component that holds the class. Already written, behind a switch.
2. **A short definition's modifiers from a constant.** Already worked.
3. **A short definition's modifiers naming the holder's parameters.**
   The filled-in inputs were remembered as written, so `c = k` reached
   the run as a bare `k` that nothing answered for. They now carry the
   flat model's names, which is the invariant this repository states
   for anything surviving flattening.
4. **The order the components are written in.** An `outer` reaches the
   shared instance from anywhere, and `User u; inner World world;` is
   as legal as the other order. In that order the walk met
   `world.accel(...)` before it had ever looked inside `World`, and
   the table of filled-in inputs was still empty: `standardGravityAcceleration
is missing its argument gravityType`. The shared instance's aliases
   are now read before the components that reach it.
5. **The same order, one storey up: shapes.** With the inputs arriving,
   the value one of them carried - `g * normalizeWithAssert(n)` - named
   an array of the world that nothing had measured yet, and the call
   answered with a scalar where a vector was declared: `an equation
between shapes [3] and []`. The shared instance's array lengths are
   now taken at the same point.

Past link 5 `RevoluteConstraint` flattens and reaches a wall of another
family - `structurally singular model`, the tearing question - so the
chain has an end and this is it.

**What the series was worth: no models, and a whole family moved.**
820 flatten and 392 run on both sides, from one binary with
`OXIDELICA_COMPONENT_MEMBER_OFF` as the only difference, and the two
lists of models that ran are identical line for line.

The census is where the work shows, and it shows it whole:

```text
  16 -> 0   unknown function `world.gravityAcceleration`
   0 -> 15  subscripts and arrays survive flattening only as scalars
   4 -> 0   structurally singular: cannot differentiate function
   0 -> 4   structurally singular: cannot differentiate this expression
```

Sixteen models out of the largest row of the run half's census, and
fifteen of them standing together at one new wall, a wall that did not
exist before because nothing reached it. This is precisely the thing
the register was written to catch and no count of models can: a family
that travels one storey up entire. Had the shift read only the floors,
five walls removed would have read as a wasted day.

The zero was checked against the thing it is supposed to be able to
show, which is the rule this repository has for zeroes: on the twelve
line model the switch prints one run with the reading and none without
it, so the pipe can say something other than nothing. And the first
pair of numbers taken this shift was thrown away for the opposite
reason - both corpus runs went through a `target/release` binary built
before the change, which is the scar about two numbers being comparable
only if the same binary produced them, met from the direction of a
script that does not build what it measures.

So the row of sixteen models _was_ one family after all, which the
lookup's first measurement could not have shown: read alone it moved
three models and looked like a loss, and only the whole series shows
the sixteen travelling together. Fifteen now stand at `subscripts and
arrays survive flattening only as scalars` and one at tearing. That
split is the next shift's queue, and it is the larger half that is
newly reachable rather than the architectural one - which is the
opposite of what the map predicted, and the reason the register was
run rather than reasoned about.

### The wall the family moved to, and what stood at it

The fifteen models the last series left standing at `subscripts and
arrays survive flattening only as scalars` were probed rather than
reasoned about, and the probe took one second where the corpus takes
eleven minutes. `Pendulum` refused at `code.rs:738` with `an array of
3 written out`, and a print at that point named the expression: the
literal `{1, 1, 1}`.

That is not the gravity vector the map predicted. It is `min={1,1,1}`,
the bound on `Modelica.Mechanics.MultiBody.Types.RotationSequence`,
arriving at the run through `bound_asserts`. A bound written over a
whole array is attached to the array, and every element of that array
is its own component by the time the run is built - so the assertion
built for `x[2]` compared one number against three.

The twelve-line model showed it in one second:

```modelica
model B
  Real x[3](min = {1, 1, 1});
equation
  x[1] = 2; x[2] = 3; x[3] = 4;
end B;
```

The layer already knew the shape of this problem from the other side:
`names_an_array_maker` skips a bound written as `zeros(m)`, because a
call would reach the code generator as a function nothing knows. A
bound written out reaches it as an array instead, and the two are the
same fault in two spellings. The difference is that a written-out
bound _can_ be answered: the flat name carries the subscript, `x[2]`
is held to the second number written, and that is the assertion
Modelica means. A name with no subscript, with more than one, or with
a subscript that chooses nothing is skipped as the array makers are -
a refusal rather than a guess about which number was meant.

**What it was worth: three models, and the floors moved.** 820/392 to
820/395, runnable 722/387 to 722/390. The diff of the run lists is
three additions and no removals:

```text
> Modelica.Mechanics.MultiBody.Examples.Elementary.Pendulum
> Modelica.Mechanics.MultiBody.Examples.Elementary.UserDefinedGravityField
> ModelicaTest.Rotational.TestMove
```

Two of the three are the family the last series moved here, which is
the first time this ladder has paid in models rather than in storeys.
The third had nothing to do with `MultiBody` and was reached by the
same rule, which is the sign the rule is about bounds and not about
gravity.

Twelve of the fifteen still stand at the same text, so the wall is a
wall for more than one reason, and the next probe starts where this
one did: print the expression, not the family.

## A coefficient the model writes as sometimes zero

`residual N of algebraic loop` stood at the top of the run half with
forty models under it. The kind is not a family, so the first thing
taken was the list and its spread over chapters: eleven in
`Modelica.Thermal.FluidHeatFlow`, eleven in
`Modelica.Electrical.Machines`, five in `Mechanics.Rotational`, and
the rest in ones and twos. Eleven under one chapter is one shared
component and not eleven illnesses, so `FluidHeatFlow` was the cluster
worth probing, and `TestCylinder` - a block of one unknown - was the
smallest member of it.

The probe printed the block whole, and the mechanism was in the
smallest line of it:

```text
inner cylinder2.flowPort.h :=
  -(H_flow - (if m_flow >= 0 then 0 else h*m_flow))
  / -(if m_flow >= 0 then m_flow else 0)
```

That divisor is what `semiLinear(m_flow, flowPort.h, h)` leaves behind
when the plan solves the enthalpy equation for the port's own
enthalpy. On the branch where the flow runs out of the component the
equation does not mention that enthalpy at all - the coefficient is a
literal zero - and the plan is made once for the whole run. So the
division is an infinity waiting for the model to enter the branch,
which at `t = 0` with no flow it does immediately, and what the run
then reported was a residual it could not evaluate: a refusal naming
the solver, the one place nothing was wrong.

This is the same rule as the previous series, one spelling further
out. There the slope was a number the model settled to zero; here it
is an `if` with a zero branch written into the source. Both are the
equation declining to mention its unknown, and in both cases refusing
to divide leaves the equation in the tearing set, where the connection
equality that does determine the enthalpy becomes the residual.

Only an _outright_ zero branch counts. A branch carrying some other
expression may be zero at a moment, but so may any coefficient, and
refusing on that would leave nothing solvable at all.

Reproduced in twelve lines before anything was touched:

```modelica
model Half
  Real m; Real h; Real H;
equation
  m = time - 1;
  H = if m >= 0 then h * m else 3 * m;
  H = 0;
end Half;
```

Before the change `h` runs at `inf` and ends `NaN`; after it, the
compiler says `singular Jacobian in algebraic loop ["h"]`, which is
the truth about a model that pins `h` only where the flow is positive.

**What it was worth: seven models, and the floors moved.** 820/398 to
820/405, runnable 722/393 to 722/400. Seven additions, no removals:

```text
> Modelica.Mechanics.Rotational.Examples.CoupledClutches
> Modelica.Mechanics.Rotational.Examples.OneWayClutch
> Modelica.Mechanics.Rotational.Examples.OneWayClutchDisengaged
> Modelica.Mechanics.Rotational.Examples.SimpleGearShift
> Modelica.Thermal.FluidHeatFlow.Examples.IndirectCooling
> Modelica.Thermal.FluidHeatFlow.Examples.PumpDropOut
> ModelicaTest.Rotational.TestFriction
```

Worth reading twice: only two of the seven are from the chapter the
probe was aimed at. The clutches were reached by the same rule from a
quite different direction - a clutch writes its torque the same way a
pipe writes its enthalpy - which is the sign the rule is about
coefficients and not about fluids. The other nine of the
`FluidHeatFlow` eleven moved one storey up, from a residual that could
not be evaluated to `underdetermined algebraic loop`, which is an
honest statement about the enthalpy of a port nothing is flowing
through. That is the next wall in that chapter, and it is a different
question: what an initial equation should pin when the flow is zero.

## A declaration a base already wrote, word for word

The census of the run half, after the branched-slope change, ranked
`unbalanced model` first at 137 once rows of the same meaning were
added together. But the probe, not the counter, found the largest
cluster that was actually one layer: twenty-four models of the
quasi-static magnetic and electrical chapters, split by the counter
across `unbalanced model` (11) and `two equations for der(...gamma)`
(13) - one family wearing two wordings, which is exactly the blind
spot the register is known to have.

The smallest member of it, `EddyCurrentLosses`, showed the mechanism
in one second. `TwoPortElementary` declares

```modelica
SI.AngularVelocity omega = der(port_p.reference.gamma);
```

and `EddyCurrent`, which extends it through `TwoPort`, declares the
very same line again. We kept both. The language says an element
inherited more than once is included once; two copies of a declaration
with a binding are two equations for one derivative, and the model
cannot be run at all.

The smallest model that fails the same way is eleven lines:

```modelica
model Base
  Real w = der(g);
  Real g;
end Base;

model Dup
  extends Base;
  Real w = der(g);
equation
  g = 2.0 * time;
end Dup;
```

The rule taken is narrow on purpose: a declaration is a repetition
only when the base says the same thing in every respect the compiler
can compare - the same type, the same variability, the same causality,
the same dimensions, the same binding, and a binding actually present.
A class that changes any of those is saying something, and what it
says is not for this rule to swallow. A `redeclare` and a `replaceable`
are both left alone for the same reason.

**What it was worth: no model, and a family one storey up.** 820/405
before and after, runnable 722/400 both ways, and the two run lists are
identical line for line - no addition and no loss. The register is the
only instrument that shows the work: `two equations for der` went from
16 to 4 and `unknown variable` from 16 to 27, with `unbalanced` moving
137 to 138. Twelve models travelled past the wall this change removed
and stopped at the next one, which for the machine chapter is
`unknown variable imcQS.vs[1]` and for `EddyCurrentLosses` is an
unbalanced count. That is the next question in the chapter, and it is
a different one.

## Where the time goes, asked from one run

A shift was spent hunting a slowdown with two binaries and two runs of
the corpus, and the numbers the hunt wanted were already being
gathered - they were simply never printed by name. `library check`
knows what each model cost, in both halves, and until now it totalled
them and threw the detail away.

`--slow N` prints the N dearest models with their seconds, the
flattening half and the run half apart. Measured on the whole library
at 820 flatten / 405 run:

```text
  dearest     65.7s  Modelica.Electrical.Spice3.Examples.Spice3BenchmarkFourBitBinaryAdder.FOURBIT
  dearest     53.0s  Modelica.Mechanics.MultiBody.Examples.Loops.Engine1a
  dearest     45.2s  Modelica.Mechanics.MultiBody.Examples.Loops.Engine1b
  dearest     44.0s  Modelica.Mechanics.MultiBody.Examples.Loops.Engine1b_analytic
  dearest     42.5s  ModelicaTest.Fluid.TestComponents.Pipes.DynamicPipeEnergyConservationCheck2
  slowest run 55.8s  Modelica.Mechanics.MultiBody.Examples.Elementary.RollingWheelSetPulling
  slowest run 53.9s  Modelica.Mechanics.MultiBody.Examples.Elementary.RollingWheelSetDriving
  slowest run 53.9s  Modelica.Mechanics.MultiBody.Examples.Loops.Engine1a
  slowest run 39.3s  Modelica.Mechanics.MultiBody.Examples.Loops.Fourbar2
  slowest run 33.2s  Modelica.Mechanics.MultiBody.Examples.Rotational3DEffects.GyroscopicEffects
```

Two things the totals could not have said. The cost sits in a handful
rather than spread over hundreds: twenty models out of 1043 carry
about a third of the flattening half between them. And the two halves
rank differently - `FOURBIT` is the dearest thing in the library to
flatten and does not appear in the run half at all, while the rolling
wheel sets are the dearest to run and nowhere near the top of the
other list. A single "dearest" list would have named one and hidden
the other.

It also corrects a note that has stood in AGENTS.md for some time.
`DoublePendulum` was written down as the known giant at forty-five
seconds; it is 23s to run and not in the flattening list, and the
things above it are the wheel sets, the engines and the Spice
benchmark. MultiBody owns nearly the whole ranking either way, which
makes it a chapter rather than a model.

**A ceiling on the time per model, beside the five counts.** The other
half of the same instrument. The floor script held five counts and
reported the time without holding it to anything, which is what let a
regression take the run half from 591s to 3153s and go unnoticed for
two days, until it killed two build machine runs on the ninety minute
cap. What is held is the time per model that reached each half, not
the total: the total growing is coverage growing, which is the point
of the work, and the per-model number growing is the compiler getting
slower, which is not.

The pair is written from the build machine's numbers rather than a
desk's - 4143ms and 2657ms there against 1666ms and 889ms here, a
factor of two and a half - which is the opposite way round from the
counts, and for the same reason: a threshold belongs where the machine
that fires it can reproduce it. The ceilings are 7000ms and 4500ms,
about two thirds of headroom over the dearest run seen, because two
runs of the same code on the same machine already differ by a seventh
and this is a trap for a factor of five, not a benchmark.

## The quasi-static reference angle: a chain walked, and parked

Twenty-one models of the library are stopped by the same name. Taken
from `library check .msl --refused` at 820 flatten / 405 run, every
model whose refusal quotes a `gamma`:

```text
Electrical.Machines.Examples.InductionMachines.IMC_DOL
Electrical.Machines.Examples.InductionMachines.IMC_Inverter
Electrical.Machines.Examples.InductionMachines.IMC_Steinmetz
Electrical.Machines.Examples.InductionMachines.IMC_Transformer
Electrical.Machines.Examples.InductionMachines.IMC_YD
Electrical.Machines.Examples.InductionMachines.IMC_YDarc
Electrical.Machines.Examples.InductionMachines.IMS_Start
Electrical.Machines.Examples.Transformers.IMC_Transformer
Electrical.QuasiStatic.Polyphase.Examples.BalancingDelta
Electrical.QuasiStatic.Polyphase.Examples.BalancingStar
Electrical.QuasiStatic.SinglePhase.Examples.ParallelResonance
Magnetic.QuasiStatic.FluxTubes.Examples.BasicExamples.QuadraticCoreAirgap
Magnetic.QuasiStatic.FluxTubes.Examples.BasicExamples.ToroidalCoreAirgap
Magnetic.QuasiStatic.FluxTubes.Examples.BasicExamples.ToroidalCoreQuadraticCrossSection
Magnetic.QuasiStatic.FluxTubes.Examples.FixedShapes.CuboidSections
Magnetic.QuasiStatic.FluxTubes.Examples.FixedShapes.CylinderSections
Magnetic.QuasiStatic.FluxTubes.Examples.Leakage.CylinderLeakage
Magnetic.QuasiStatic.FluxTubes.Examples.Leakage.GeneralLeakage
Magnetic.QuasiStatic.FluxTubes.Examples.LinearInductor
Magnetic.QuasiStatic.FluxTubes.Examples.NonLinearInductor
Magnetic.QuasiStatic.FundamentalWave.Examples.Components.EddyCurrentLosses
```

The smallest, `EddyCurrentLosses`, answers in a second, and what it
says is that the model is short of equations rather than over:
2427 for 2485 unknowns, with `loss_e.plugToPins_p.plug_p.reference.gamma`
among the fifty-eight named by nothing at all.

**Link one: a port of ports may hold more than ports.** A `connect`
between two connectors that each hold connectors is cut open into
pairs of the inner ones, because writing the equations against the
outer pair would name something the flat model does not carry. That
cut is right about the inner ports and wrong about everything else a
port may hold beside them: a quasi-static plug carries `pin[m]` _and_
a reference angle, and cutting the join into the pins alone dropped
the angle on the floor. The repair is to join the outer pair as well
and have the equality loop pass over members that are themselves
ports, which their own pairs already speak for.

Measured on `EddyCurrentLosses`: 2427 equations became 2507 against
2485 unknowns, and the fifty-eight names that nothing determined are
determined. The model is now *over*determined by twenty-two, and the
twenty-two are all reference angles.

**Link two, which does not fit: the graph is empty where the loop
is.** Twenty-two too many is what section 9.4 exists for - a
connection closing a loop of the overconstrained graph owes the
record's `equalityConstraint` rather than an equality, which for the
reference angle is a residue of no elements. Breaking the graph open
with a spanning tree from the chosen roots and keeping only the
tree's equalities was written and measured, and it cuts forty-eight
where twenty-two are wanted: the model goes to 2459 for 2485, short
again by a different number.

The reason is in the library rather than the compiler.
`Polyphase.Basic.PlugToPin_p` is the component that fans a plug out to
a pin, and its `Connections.branch` clauses are _commented out_ in the
standard library source, with the equality left standing outside them:

```modelica
  //Connections.potentialRoot(plug_p.reference);
  //Connections.potentialRoot(pin_p.reference);
  plug_p.reference.gamma = pin_p.reference.gamma;
```

So the graph the compiler can see has no edges across exactly the
components where the ring closes, and a spanning tree over it cannot
tell a loop-closing connection from any other. The tree drops
equalities the model needed and keeps ones it did not.

**Parked, with the map.** Link one is a real repair sitting on a
second that does not yet exist, and the charter is explicit that a
chain is taken whole or not at all: a link removed from the middle
moves no number by construction. Neither link is committed. What the
next shift needs, in order:

1. Which equalities are the twenty-two. The probe is
   `library check .msl --refused --only ...EddyCurrentLosses` with
   link one applied, whose message names them: they are
   `plugToPin_p[2].plug_p.reference.gamma = plugToPin_p[1]...` and
   its fellows, one per phase beyond the first.
2. Whether the dependency can be seen without the `branch` clauses the
   library did not write - the plug fans out to `m` pins, all `m`
   equalities say the same angle, and `m - 1` of them are dependent
   by the shape of the fan rather than by the graph.
3. Whether that is a rule this compiler should have at all, or whether
   the honest answer is that the model is overdetermined as written
   and the standard library leans on a tool reading the commented-out
   clauses some other way. This is the question for a consultation:
   it is architectural, and three local attempts moved the failure
   rather than removing it.

The small model that shows link one is thirty lines and lives in the
shift notes rather than the tests, since committing a test for an
uncommitted repair would be a test that does not fail.

## The quasi-static reference angle: the chain taken whole

The map of the previous shift had a wrong link in it, and the error is
worth keeping: `PlugToPin_p` was read as having its `Connections.branch`
commented out, and the architectural question was built on that reading.
The branch is there, on the first line of the equation section; what is
commented out are two `potentialRoot`s below it, and the file with the
branch truly absent is `PlugToPins_p`, which is dead legacy. The rule
that follows is cheap: read the whole file before quoting it, because
`grep -n Connections` over one file costs a second and this cost a shift.

With the branches present the chain has three links, and it was taken as
one series.

**Link one: a port of ports may hold more than ports.** A `connect`
between two connectors that each hold connectors was cut open into pairs
of the inner ones, which is right about the inner ports and silent about
everything else the port holds: a quasi-static plug carries `pin[m]` and
a reference angle, and the angle was dropped on the floor. The outer pair
is now joined as well, and the equality loop passes over members that are
themselves connectors, which their own pairs already speak for. On
`EddyCurrentLosses` that took 2427 equations for 2485 unknowns to 2507 -
the fifty-eight undetermined angles determined, and twenty-two too many.

**Link two: a connection of the graph is an edge, not an equality.** By
section 9.4 a connection between connectors carrying an overconstrained
record contributes an edge, and a spanning tree decides what each edge
comes to. The branches the components wrote are the tree's by right, and
each carries the equation its own component states - `PlugToPin_p` writes
`plug_p.reference.gamma = pin_p.reference.gamma` itself. So the sets are
walked in order, each in-graph member tied to what the ones before it
built, and a member already tied is the one closing the loop: its
equality is what `equalityConstraint` replaces.

**Link three: only where that constraint is empty.** For the reference
angle the constraint returns `Real residue[0]` - no equations, so the
connection simply goes. A multibody `Orientation` returns `residue[3]`,
three equations this compiler does not yet write, and dropping the
equalities there takes equations away and puts none back. Measured: with
the rule ungated the corpus fell to 400 run from 405, and the five lost
were all MultiBody. Gated on the residue's length they came back.

**And a fourth that only the corpus could show.** Joining the outer pair
is wrong for an expandable bus, whose members are all signals and all
connectors in their own right: the pair of buses states every one of
them a second time. Two `PathPlanning` models were lost that way and
nothing smaller showed it - the fault needs a bus with a dozen members
to appear at all. The outer pair is joined only where the port is not
expandable and carries something that is not one of its own ports.

The numbers of the whole chain, from one run of the script: 820 flatten
and 405 run before, 820 flatten and **407 run** after, with the diff of
the run lists showing no withdrawals at all and two additions,
`FluxTubes.Examples.FixedShapes.CuboidSections` and `CylinderSections`.
The floors move to 407 and 402 in the same commit.

What did not come: `EddyCurrentLosses` itself, which was predicted to
reach 2485 on 2485 and stands at 2505. Nineteen loop-closing connections
are still being written as equalities somewhere the set walk does not
see, and that is where the next attempt starts. The prediction is
recorded as missed rather than adjusted.

### Link five: a pass-through is a node of the graph

The nineteen the previous attempt could not account for were found by a
probe rather than by reasoning about shapes: a counter printed, for each
equality the set walk writes about a graph record, whether the walk
considered the node to be in the graph at all. It named the line in one
second. The nodes were gathered from the `branch` and `root` clauses
alone, so a connector no clause names was outside the graph entirely -
and `PlugToPins_p` is exactly that, an outer plug joined to an array of
inner converters by plain `connect` equations and writing no branch of
its own. The probe's output on `EddyCurrentLosses`:

```text
in_graph=true  loss_e.plugToPins_n.plugToPin_n[1].pin_n.reference.gamma = ...
in_graph=false loss_e.plugToPins_n.plug_n.reference.gamma = ...
in_graph=false loss_e.plug_n.reference.gamma = ...
```

A node the walk cannot see does two wrong things at once: its connection
is written as a blanket equality, and - worse - it does not carry the
spanning tree, so a connection beyond it that really does close a ring is
never recognised as closing one. Every connector of a record carrying an
`equalityConstraint` is a node of that graph whether a branch names it or
not, which is what the rule now says.

Measured, and the prediction missed again in the other direction:
`EddyCurrentLosses` went from 2505 equations for 2485 unknowns to **2476
for 2485**, and `BalancingStar` from over to 1214 for 1217. The family
has crossed from over-determined to under-determined - twenty-nine
equalities dropped where twenty were owed - so the wall it stands at is a
different one and the models still do not run. That is the shape the
notes describe: a family taken one storey up moves no count of models.

The corpus says the storey was climbed without cost: 2671 files read,
820 flatten, 407 run, runnable 722 and 402 - identical to the run before
it, and the diff of the run lists is empty name for name, no withdrawals
and no additions. The floors do not move because the numbers do not.

Where the remaining nine equalities go is the next question, and the
instrument for it exists: the same probe, printing the drops as well as
the writes, already lists them by name.

### Link six: the nine were never equalities at all

The previous link left a question with a number on it - twenty-nine
equalities dropped where twenty were owed - and the number was wrong in
a way no amount of reasoning about connection sets would have found. The
suspicion of record was that array indexes had been lost on the way to a
graph node, so `plugToPin_p[1].plug_p.reference` and
`plugToPin_p[2].plug_p.reference` were being read as one node and the
connections of different elements as a repeat of one edge.

The probe was restored on the dropped side, naming both connectors and
the node each was read as, and it refuted that outright in one second.
The indexes are intact:

```text
ring drop: loss_e.plugToPins_p.plugToPin_p[2].plug_p.reference.gamma
         = loss_e.plugToPins_p.plugToPin_p[1].plug_p.reference.gamma
ring drop: loss_e.plugToPins_p.plugToPin_p[3].plug_p.reference.gamma
         = loss_e.plugToPins_p.plugToPin_p[1].plug_p.reference.gamma
```

Better than reading the thirty-one names: the probe also prints the
branch clauses and the equalities it does write, so the graph can be
built outside the compiler and its cyclomatic number taken. Over 242
nodes, 86 branch edges and 185 connection edges spanning two components,
the number of independent loops is **31** - exactly the count of drops.
Every one of the thirty-one was a genuine loop closure, and the ring
machinery had nothing left to answer for. A small model written to the
predicted shape - a wrapper holding an array of three pass-throughs -
ran green without any change at all, which said the same thing from the
other end.

So the deficit of nine was never a shortage of equations. It was a
surplus of unknowns, and the refusal had been naming them all along:

```text
nothing determines loss_e.omega, powerb_e.omega, resistor_e.omega, ...
```

Seventeen of the nineteen were `omega`, and `oxidelica why` showed the
same variable declared twice in one component. The smallest member of
the family said it plainest -
`SinglePhase.Examples.SeriesResonance` at 129 equations for 133
unknowns, where the whole chain is two classes deep:
`TwoPinElementary` declares a bare `SI.AngularVelocity omega` and states
it with `omega = der(pin_p.reference.gamma)`, and `TwoPin`, which
extends it, declares the identical bare `omega` again.

That is the shape link four of an earlier chain already fixed - a
declaration a base wrote word for word is one element - except that the
rule asked for a binding to be present, and these copies have none. The
rule now asks instead that the two declarations agree in every respect
that can be compared, the attributes a modifier may carry among them: a
class that changes a `start`, a `min` or a unit is saying something, and
only a word-for-word repetition says nothing.

The instrument ladder is the finding as much as the fix is. The corpus
would have said nothing; the census counts refusals and this family had
one refusal, unchanged in wording through three shifts. What answered it
was `--only` on one model in under a second, then `why` on one variable,
then the same on the smallest member of the family instead of the
largest. Three shifts were spent on a ring machinery that turned out to
be correct, because the count of dropped equalities looked like an
accusation and nobody took the graph's cyclomatic number to check it.
A number that looks like evidence is not evidence until something
independent produces it.

The corpus is the first number this chain has moved since it began.
2671 files read, 820 flatten as before, and **421 run against 407**;
runnable 722 flatten and 416 run against 402. The diff of the run lists
has no withdrawals and fourteen additions, and they are the family and
its neighbours - `SeriesResonance`, `ParallelResonance`,
`MultipleResonance`, `SeriesBode`, `BalancingStar`, `BalancingDelta`,
`UnsymmetricalLoad`, `EddyCurrentLosses`, `PolyphaseInductance`, two
`ToroidalCore` models, and three `HBridge` converters that carry the
same quasi-static interfaces. Both floors move in this commit.

## An input nobody wired is not a model short of an equation

The census taken over 421 running models ranks the run half by kind,
and the rows have to be added before they are read: `unbalanced model`
carries 116 of them, ahead of `cannot evaluate parameters` at 52,
`parameter has no value` at 50 and the algebraic-loop rows at 46
together. That top row is a bush rather than a family - the machines,
the fluid vessels and the operational amplifiers each stand at it for
their own reason - and the way in was the smallest member by equation
count rather than the most interesting by name.

The smallest was `StateGraph.Examples.Utilities.Source` at one
equation for two unknowns, and twelve lines reproduce it whole:

```modelica
model TopInput
  connector Outflow output Real out; input Boolean open; end Outflow;
  Outflow outflow1;
equation
  if outflow1.open then outflow1.out = 1; else outflow1.out = 0; end if;
end TopInput;
```

`outflow1.open` is an `input` that no `connect` names. Nothing inside
the model writes it, and nothing was ever going to: an input is
supplied from outside, exactly as an unconnected `flow` carries
nothing. The compiler counted it as an unknown and refused the whole
model for a value that was never the model's to find. The same for a
connector that is one value rather than a set of members -
`connector RealInput = input Real`, which is how every signal of the
standard library is written - and that second layer is why `valve` and
the mixing-unit blocks stood at the same wall one storey up.

The first cut of the fix gave every set of one its start value, and
the corpus answered 416 against 421: five models lost, twelve
withdrawn and seven added. The withdrawals name the blind spot. A set
of one is not the same thing as a connector nobody joined: a port its
own class joins from the inside and the level above leaves alone
stands as a set of one too, and what it is short of is the seam rather
than a value. Giving it one wrote over what the class inside already
states, and `Modulation` came out with 37 equations for 27 unknowns -
ten equations too many. The narrowed rule asks that no `connect`
anywhere names the connector, which is a set the flattener already
computes for the unconnected-flow zero.

Narrowed, the corpus reads 2671 files, 820 flatten, **428 run against
421**; runnable 722 flatten and 416 run, unmoved. The diff of the run
lists has no withdrawals and seven additions: `Source` and `Tank` of
the StateGraph utilities, `CriticalDamping`, `MixingUnit`,
`IntakeManifold` and `TorqueGeneration` of the clocked systems, and
the batch plant's `Controller`. Only the total run floor moves; the
runnable pair is untouched, because all seven are utility models
rather than examples with an experiment.

Worth recording as method: the first measurement was the useful one
precisely because it went down. A change that had only been checked on
the family it was written for would have shipped ten spurious
equations into every model with a port on a boundary, and no test in
the tree asks that question - the diff of the run lists is the only
instrument that names five diffuse losses, and it named them by
model in one pass.

## A block asked for on its own has no level above it

The wall the machines stand at was taken the way the previous one was:
by the smallest member of the family rather than the loudest. Of the
thirty-two `unbalanced model` refusals from Machines and Magnetic,
`ControlledDCDrives.Utilities.LimitedPI` is the smallest at 31
equations for 33 unknowns, and `why` named what the census could not:

```text
in ...Utilities.LimitedPI, about `controlError`:
  equation: controlError = u - u_m
in ...Utilities.LimitedPI, about `integrator.u`:
  equation: integrator.u = addAntiWindup.y
```

Both names the refusal called undetermined have equations of their
own. Two equations were missing rather than two names unwritten, and
the two are `u` and `u_m` - the block's own inputs. Twelve lines
reproduce it:

```modelica
block Wrap
  RealInput u; RealOutput y; Gain gain;
equation
  connect(u, gain.u);
  connect(gain.y, y);
end Wrap;
```

`u` is joined, so the rule of the previous shift - a connector no
`connect` names anywhere - does not reach it. But it is joined
_inward_: the block hands its own input to the component doing the
work, and the equality was written pointing the wrong way, from the
port into the model. Nothing then wrote the top-level name at all. A
block asked for on its own is the whole of the run, and the level
above that would have supplied the value is not there.

So a top-level `input` states its set, before the ordering by
causality that the set otherwise uses, and where the set has no other
source it stands at its own declared start.

Two guards were bought with red tests rather than reasoning, and each
is a rule about which question is being asked of which name:

- Causality is asked of the _name's own_ connector, never of the
  set's. A set joins an output to an input, so one of the two classes
  is picked to describe the set and would answer for the other member
  as well: asked of the set, the `y` of a block whose `u` is a signal
  was read as an input and given a value on top of the one the model
  computes. `LimitedPI` came out at 34 equations for 33 unknowns.
- An `output` inside the set comes first all the same. A set holding
  one is already told what to carry, and a top-level input joined to
  it is reading that value rather than waiting on one. So is a set
  holding a name the model states outright - a source's `y = 2 *
time`, written by its own class rather than declared as an output.
  Either way the port must not also take its start, or the set has two
  definitions and one of them has nothing left to determine.

The corpus reads 2671 files, 820 flatten, and **445 run against 428**;
runnable 722 flatten and 416 run, unmoved, since all seventeen
additions are utility models rather than examples with an experiment.
The diff of the run lists has no withdrawals. Beside `LimitedPI` the
seventeen are the inverse-model utilities of four chapters at once -
`DirectInertia` and `InverseInertia`, `DirectMass` and `InverseMass`,
`InverseCapacitor` and `InverseInductor`, `DirectCapacity` and
`Conduction` - which are written as blocks taking a signal in and are
exactly the shape this rule describes. The run floor moves here.

The machines themselves did not move, and that is the expected half:
`TranslatoryArmatureAndStopper` and `SpeedControl` are still refused,
one wall further along. The census entry loses this storey rather than
the family.

## A state the initial section says nothing about

An `initial equation` section was counted against every state of the
model, so a model with nine states and a section written about two of
them was refused as an initialisation that is not square. It was the
tallest entry in the run half of the register, thirty-three models,
and the model that names it plainest is four lines long: two states,
one initial equation, and a refusal that the problem has one equation
for two unknowns.

It has two. The section can only move the states it mentions;
whatever it is silent about is not an unknown of the initialisation
at all, and stands at the start value it was declared with. Filling
those in is what makes the problem square, and it is the same rule
the language states - a start value is a guess only where something
else has an opinion about it.

Mentioned is asked of the whole section rather than of each equation,
because an equation relating two states pins neither on its own and
both are unknowns of the system the section forms. And the filling in
is allowed only where it makes a lopsided problem square, never where
the problem was square already: a section that pins nothing, or one
written about an algebraic variable, has its own diagnosis waiting
below, and pinning the states it did not mention would answer that
with an arithmetic complaint instead. Both restrictions were bought
by red tests.

The corpus reads 2671 files, 820 flatten, and **450 run against 445**;
runnable **421 against 416**. The diff of the run lists has no
withdrawals, and the five are `DCPM_Drive`, `LinearInductor` and
`NonLinearInductor` of the quasi-static flux tubes, `FreeBody`, and
`Vehicle`. Both run floors move here.

The solenoid comparisons are the other half of the entry and they
moved a storey rather than out: `ComparisonQuasiStatic` now stands at
a singular Jacobian in an algebraic loop and `ComparisonPullInStroke`
at a residual of one. That is the expected shape - the tallest entry
of the register drains, and what stood behind it is a numerical wall
of its own.

## A parameter the initialisation was left to solve

The entry above drained to twenty-two, and what was left under the
same wording was a different fault. `TwoMasses`, the two-mass
conduction demo, declares the temperature its masses settle at as a
parameter the declaration does not give a value to:

```modelica
parameter SI.Temperature T_final_K(fixed = false);
initial equation
  T_final_K = (mass1.T*mass1.C + mass2.T*mass2.C)/(mass1.C + mass2.C);
```

The parameter rounds settle what they can before the run: a name
against a number, then an ordinary equation that defines one, then a
residual in a single unknown solved by Newton. This equation answers
to none of them, because the side opposite the parameter names two
states, and no round before the run knows what a state is worth. So
the parameter fell through to the rule that a start stands where
nothing else decides - and the section was then counted against the
states alone, one equation for two unknowns, refused as an
initialisation that is not square.

It is square. `fixed = false` is the statement that the declaration
is not where the value comes from, and a parameter nothing else
settled is therefore the section's own unknown, to be solved for
beside the states rather than ahead of them. The unknown vector is
widened by those parameters, each written into its slot before the
residual is evaluated so that the equations reach it the way they
reach any parameter, and the solved value goes back into the slot and
the reported parameter list, because a parameter solved for is a
parameter for the whole run. The start it was declared with is not
discarded: it becomes the guess Newton begins from, which is what a
start is for.

The refusal now names what it counted, states and parameters apart,
rather than calling every unknown a state.

The corpus reads 2671 files, 820 flatten, and **451 run against 450**;
runnable **422 against 421**. The diff of the run lists has no
withdrawals and one arrival, `TwoMasses`. Both run floors move here.

The register is the instrument that shows the rest, and the counts
are honest about it: the not-square entry goes 22 to 18 while one
model runs, because the other three moved a storey. All three are the
lightning models - `DemonstrateLightning`,
`LightningLosslessTransmissionLine`,
`LightningSegmentedTransmissionLine` - and all three now stand at an
initialisation that is singular, the entry for which fills by exactly
the three it lost. The flatten half of the register is identical line
for line, which is what a change confined to the run half owes.

## A stream that carries nothing still says which enthalpies are equal

`semiLinear(m, h_a, h_b)` is `h_a*m` where the flow runs one way and
`h_b*m` where it runs the other, and the two meet at zero. They meet
rather too well: at `m = 0` the expression is zero whatever the
enthalpies are, so the equation that carries it determines neither of
them. A chain of components joined end to end therefore has a chain of
enthalpies that nothing pins down the moment the flow through it
stops, which is the state most of these models start in.

What the solver made of that was three different refusals for one
cause. Where the finite differences cancelled cleanly the block was
`underdetermined`; where they left noise the same block was a
`singular Jacobian`; and where an explicit assignment had already been
minted by dividing through the vanishing slope, the first residual was
not a number at all. The census reads these as three families, and the
probe reads them as one wall - which is the gap between counting kinds
and probing layers, seen again.

The language says what the missing equation is (3.7.2.5): where the
flow is zero the two enthalpies are equal, since nothing is being
carried and the mixing has no direction to prefer. So the equation
becomes a conditional on the flow, with the transport term while the
flow is nonzero and the difference of the two enthalpies at the moment
it is not. Nothing is lost by the swap: at zero flow the transport
equation says `H = 0`, which the other side of it already says.

Three things about the shape were learned by getting them wrong, and
each is held by something.

The rewrite runs _after_ the checks rather than while the equations
are written. Its two branches carry different dimensions - a
difference of enthalpies against a flow of energy - and the unit layer
is right to say so of anything a model writes. What is written here is
not a model's expression but a plan for solving one, and a plan is not
what the dimensional check is over. Put before the layer, it turned
nine models from a bad answer into a refusal about units.

Only where both enthalpies are plain names. `semiLinear(u, 2, 5)` is a
pair of slopes written out, both settled already, and rewriting it
says `y = 2 - 5` where the meaning is `y = 0` - a wrong number in
place of a right one, which is the worst thing this compiler can do.
A test in the flattening suite had this covered and went red within
the minute, which is the cheapest possible news.

And the transport term is swapped _for_ the difference rather than
added to it. Carrying both keeps the branch an identity in the
transported quantity, which reads well and costs everything: with
nothing solvable there, every member of the family fell back to an
infinite residual. Measured on nine models, the added form won none
and the swapped form won three.

The corpus reads 2671 files, 820 flatten, and **454 run against 451**;
runnable **425 against 422**. Both run floors move here. The diff of
the run lists, taken from one binary with the change switched off and
on, has no withdrawals and three arrivals - `OneMass`, `TwoMass` and
`ParallelCooling`. The other six of the family moved a storey rather
than through the door: they stand now at an infinite first residual,
where the enthalpy is determined but something upstream of it is not
a number, and that is a different wall in a different layer.

## A state the declaration is silent about does not start at zero

`Modelica.Thermal.FluidHeatFlow` was left with nine models standing at
three different walls after the `semiLinear` work of the last shift:
two at a singular Jacobian, one underdetermined, and three at an
infinite first residual. The probe read the three residual ones first,
because a loop of one variable is the smallest thing in the family,
and what it found was not in the solver at all.

`TestCylinder` joins two cylinders of fluid. Each writes its mass as a
volume, `m = rho*A*s`, and carries its energy as a total, `H = m*h`,
from which the specific enthalpy is got back by dividing. The
declaration gives `s` a start and `m` none - there is nothing to give
it, the equation already says what it is. But a state with no `start`
began at zero, so the mass began at zero, so the division that
recovers `h` was a division by zero at the very first step, and every
enthalpy downstream of it was NaN before Newton had taken a step.

The compiler already knew how to read a start out of an equation: it
did it for the torn variables of an algebraic block, where a bad guess
costs convergence. What it did not do was read one for a _state_,
where a bad guess is not a guess at all - a state's start is where the
run begins, and nothing later writes over it.

Three things about the shape were learned by measuring, and two of
them by getting the cost wrong.

The reading has to chain. A temperature gives an enthalpy, the
enthalpy and a mass give a total: one pass over the equations gets
whichever link happens to come first and leaves the rest at zero, so
the pass repeats until nothing more can be learned, and the value
learned is written back into what the next round may read from.

The chaining has to be bounded, and so does what it is asked about.
Unbounded, with every name in the model a candidate, the corpus went
from eleven minutes to twenty-five; bounded in rounds but still asked
of every name, it was eighteen. What makes it cheap is asking only the
names within one equation's reach of something that actually wants a
start - the torn variables and the silent states - which is a set
worked out once, by naming alone, with no solving and no evaluation in
it.

And a slow measurement is worth doubting before the change is blamed.
One of those numbers was not the change: a timing run left behind a
stray process eating eight cores, and with it gone the model measured
77 seconds with the change switched off and 77 with it on.

## A torn block that divides by its own unknown

The largest entry of the run half's census was `residual N of algebraic
loop ... is NaN at t = 0, before any Newton step`, twenty-six models,
and the wording put the fault in the solver. The probe put it in the
plan. Nineteen of the twenty-six said NaN, six said `-inf` and one
`inf`, which is one kind of arithmetic and not three.

The smallest of them, `Modelica.Electrical.Analog.Examples.Resistor`,
is twelve lines of physics: a heated resistance whose value depends on
a temperature the dissipated power drives. Its loop holds `i`, `R_actual`
and `T` together, and `oxidelica why` said `R_actual` had no binding, a
start of nothing, and two equations naming it. The plan probe said the
rest: the block's inner chain was

```text
inner i        := -Q / -v
inner R_actual := -v / -i
inner T        := ...R_actual...
```

An inner assignment of a torn block is evaluated before Newton has
moved anything, at whatever its divisor happens to hold - and an
unknown of the block holds its declared `start`, which is zero unless
a declaration said otherwise. So `R_actual := v/i` divides by zero on
the first evaluation, every time, and every value below it in the chain
is NaN before a step is taken. The refusal then names the solver, which
is the one place nothing was wrong.

The fix is to keep such an equation out of the explicit set, so that
Newton carries it with the rest of the block and no division by a
starting value is done at all. The first two attempts at the rule are
the finding.

**A chain of two links, not one.** Refusing only a divisor belonging to
the same block moved the wall one step: the divisor became `v`, an
explicit algebraic that a sine source makes zero at `t = 0`. The rule
had to reach any live unknown, settled or not, and taking one link
alone would have measured zero.

**A rule right almost everywhere cost six models.** Refusing every live
divisor ran the corpus at 455 against a floor of 459, and the diff of
run lists named the victims: four `ToroidalCore*` and two inductors, all
of FluxTubes. The plan probe, run twice from one binary, named what had
been dropped in one line - `coil.L_stat := if abs(i) > eps then Psi/i
else L`. The library had guarded its own division, and a guarded
division cannot be reached where it would fail. Refused along with the
rest it grew the Newton system until those blocks came back singular.

So the rule asks two things rather than one: the divisor is an unknown
whose start is certainly zero, and nothing the model wrote stands
between the division and that zero. With both, the corpus went 459 to
460 and the runnable pair 430 to 431 - two models gained,
`Resistor` and `HeatingMOSInverter`, and one lost,
`Modelica.Magnetic.QuasiStatic.FluxTubes.Examples.NonLinearInductor`.

That loss is the next link of the chain and is left named rather than
chased: its refusal is `-inf` from `r_mFe.B = r_mFe.Phi / r_mFe.A`,
where `r_mFe.A = r_mFe.area` is a parameter equality the matching gave
to `A` as an equation instead of settling it to a number. The divisor
there is not a live unknown at all - it is a constant the plan failed
to recognise, which is a different layer from this one.

The census entry is the measure of the rest: twenty-six models stand at
this wall for other reasons, counted after the move rather than before
it. MultiBody owns eight of them through `z_a`, which is a family of
its own.

## The residual nothing could evaluate, mapped

The census taken after the divisor rule put the residual wording at
twenty-seven, not the twenty-five the previous shift predicted: two
models arrived rather than one, and the two rows that emptied by one
apiece say where from - `algebraic loop diverged` and an initial value
fixed against its constraints. The prediction had counted the traffic
one way only. The previous note's "remaining twenty-four" was the same
error and is corrected here rather than in place: what stood at the
wall after that shift was twenty-six.

Named, the twenty-seven are three families and two singles. Seven are
magnetic, where a reluctance is written `R_m = 1/G_m` and `G_m` is an
unknown whose declaration left it at zero. Seven are MultiBody, through
`z_a` and the wheels' `der_angles`, where the value that is not a
number is an inner assignment of the block rather than the residual
itself. Eleven are the machines, all reading `airGap` in a block of
thirty-odd unknowns. `DrumBoiler` and a reversing valve of ModelicaTest
stand alone.

The two family sizes were each written as eight the shift the map was
made, which sums to twenty-nine against a row that says twenty-seven,
and a map that does not add up is not one work can be chosen from. The
sizes above are counted from the named list rather than from memory:
the raw report is now written beside the census by the same run, so
counting the members of a family costs nothing over counting the
kinds, and there is no longer a reason to take a family's size from
what the last note said it was.

The magnetic eight are the layer this shift took. A block whose
residual is not a number at the point it starts from has not diverged:
it was never evaluated, and the question it was asked - the reciprocal
of zero - is one neither the model nor the plan is at fault for, since
the division stands in the equation rather than in an assignment the
plan chose. So the block is started again, from each of a handful of
magnitudes in turn, and the first start that solves is the answer. Only
that one refusal is retried, and the test that says why is
`der(x)^2 = 4`: a block that converged on an ambiguous solution has
been evaluated and has something true to say, and a retry would hand
back whichever root it happened to land on.

The corpus is unmoved at 820 flatten and 460 run, runnable 722 and 431,
and the reason is the link behind this one. All eight now reach Newton
and all eight are refused a step later: `leakage.G_m` for a singular
Jacobian, `r_mFe.mu_r` and the quasi-static inductor the same. That is
the third link of the chain - the leakage element writes
`(1 - c_usefulFlux)*R_m = c_usefulFlux*R_mUsefulTot` beside
`R_m = 1/G_m`, and what the corpus says about it is that the pair does
not pin the block down as the compiler tears it. The work is a link
taken on a mapped chain rather than a model won, and it is recorded as
that.

## An unknown that stands only under a division

The chain's fourth link, and the one that was actually load-bearing.
The magnetic block refused for a singular Jacobian, which reads as a
model the compiler cannot be blamed for; probed, what stood there was
a block that should never have been torn at all.

`R_m = 1/G_m` is how every reluctance in the library is written, and
it is linear in the reciprocal and in nothing else. Linear solving
cannot reach `G_m`, so the equation joined the tearing set, and Newton
was handed a block of one whose derivative is `-1/G_m^2` - enormous
beside the pole, flat away from it. The retry of the shift before
started it off the zero at four magnitudes in turn, and all four
walked outward to where the slope dies away faster than the residual
does. The refusal named a singular Jacobian about an equation with one
plain answer.

Solved for the reciprocal and inverted there is no iteration at all.
With `u = 1/var`, an equation linear in `u` has a closed form, and
`var = 1/u`. The substitution is refused wherever the variable stands
anywhere except as a whole divisor: multiplied in as well, the
equation is quadratic once the reciprocal is introduced, and a
linear-looking answer there would be a wrong number where a refusal is
owed. So this widens what can be solved without claiming anything the
substitution does not support.

Measured from one binary with `OXIDELICA_NO_RECIPROCAL` as the switch,
the corpus went 460 to 462 and the runnable pair 431 to 433. The two
are `Magnetic.QuasiStatic.FluxTubes.Examples.BasicExamples.QuadraticCoreAirgap`
and `Magnetic.QuasiStatic.FluxTubes.Examples.Leakage.GeneralLeakage`,
and the diff of the run lists shows nothing lost against them. The
test is `an_unknown_that_stands_only_under_a_division`, which checks
the permeance comes out a sixteenth rather than checking that
something was solved.

The chain is not finished, and its fifth link is now visible rather
than guessed at. The transient twin of one of the two won,
`FluxTubes.Examples.BasicExamples.QuadraticCoreAirgap`, no longer
refuses about `leakage.G_m` at all: it refuses as an underdetermined
loop over
`leakage.Phi`, the two coils' voltages and a pair of flux derivatives,
which is a different layer and wants its own probe.

Of the magnetic models still standing, four stand at `r_mFe.mu_r` and
the solenoids' yoke permeances, where the reluctance is behind a
saturation curve rather than a plain reciprocal, and they are
`FluxTubes.Examples.BasicExamples.SaturatedInductor`,
`SolenoidActuator.ComparisonPullInStroke`,
`SolenoidActuator.ComparisonQuasiStatic` and
`QuasiStatic.FluxTubes.Examples.NonLinearInductor`. The fifth is the
twin of the paragraph above, and it is at a different wall
entirely - the count said five at one wall where the raw list says
four there and one elsewhere, and the raw list is now printed by the
same pass that counts, so there is no longer any reason to read the
map off a count.

## A block judged by the units its equations are written in

The fifth link, and the last of the chain. `leakage.G_m` had been
removed as a wall and the twin refused one step later as an
underdetermined loop of nine - `leakage.Phi`, the coils' voltages, a
pair of flux derivatives. Probed, the block is not underdetermined at
all. Its Jacobian inverts, and its smallest singular value is 1.3e-4
against a largest of 7.2e7: ill-conditioned, which a solver lives
with, and not a family of solutions.

What refused it was the test rather than the block. A converged block
is checked for regularity by comparing the smallest pivot Gaussian
elimination meets against the largest entry of the whole matrix, and
for this block those are 6.0e-3 and 7.2e7, so the pivot sits three
orders under a floor of 7.2 and the verdict follows. But the spread
between one row and another is the units its equations are written in,
and a magnetic circuit has no way of avoiding a wide one: a permeance
near `mu_0` and a reluctance near its reciprocal are in the same block
by construction. The test was asking about webers and amperes as much
as about the model.

Each row divided through by its own largest entry asks the question
that was meant. That is a change of units on one equation and nothing
else, so it cannot turn a soluble block into an insoluble one, and
scaled this block keeps a pivot of 5.9e-5 against a floor of 1e-7.

Two things are deliberately left alone, and the tests are what said so
rather than an argument - both were written the other way first and
both turned an existing test red within the minute. The columns are
left because a column belongs to an unknown and this Jacobian is built
by finite differences: the column of an unknown the residual does not
really depend on is not zero, it is noise near 1e-8, and divided by
its own largest entry that noise becomes a coefficient of one. With
columns scaled, `x = y + 1` beside `y = x - 1` - the same equation
twice, the thing the check exists to catch - comes back invertible.
The single row is left for the opposite reason: scaled, its one entry
is always one, so no block of one could ever read singular again and
`1/x = 0` would be answered with a number instead of a refusal. Rows
carry the units; the columns carry the evidence.

Measured on the corpus, it went 462 to 466 and the runnable pair 433
to 437, with nothing lost against the four. One of the four is the
twin the link before had uncovered,
`FluxTubes.Examples.BasicExamples.QuadraticCoreAirgap`, and the chain
of five is finished with it. The other three were not predicted and
are the finding: `QuasiStatic.FluxTubes.Examples.Leakage.CylinderLeakage`
is of the same family, and
`Electrical.PowerConverters.Examples.DCDC.HBridge.HBridge_DC_Drive`
and `Thermal.FluidHeatFlow.Examples.WaterPump` are not magnetic at
all - a drive and a pump, both carrying a block that mixes a
resistance with a conductance or a mass flow with a pressure, which is
the same wide spread of units arriving from somewhere else entirely.
So this was a wall across the library rather than a magnetic one, and
the probe that found it was pointed at magnetics only because that is
where the chain happened to lead. `WaterPump` had been parked two
shifts ago as underdetermined; it was not underdetermined, and the
parking list is shorter by it.

`MovingCoilActuator.ArmatureStroke` moved off this wall in the same
change without running, and now refuses about a fixed initial value
for `cActuator.x`, which is a different layer and not this family.

The `mu_r` wall is a separate matter and was probed in the same shift
without being taken. Scaled by rows, `SaturatedInductor`'s block of
three keeps a pivot of 1.3e-8 against a floor of 1e-7 - still under
it, because that block is conditioned at 1e8 in earnest rather than by
its units. So row scaling is not the answer there, and neither is the
retry off the zero: the block converges and is then refused as
underdetermined, where what is true of it is that it is
ill-conditioned. Telling those two apart wants a measure of
conditioning rather than a pivot against a floor, which is a larger
change than a shift, and the wall is parked with that as its map.

## A record is its bases' fields too: the Spice3 parameter family

`parameter ... has no value` counted fifty in the run half of the
census, the third largest row after the unbalanced models and the
parameters that will not evaluate. Twenty of the fifty were one
family: the Spice3 transistors, which work their technology parameters
out in a function and hand the answer to a `final parameter` record.
The row's wording split them - `m_oxideThicknessIsGiven`, `m_uic`,
`m_bFac` - and the layer behind all of them was one.

Three links, and the first was found by shrinking the real model to
twenty-two lines rather than by reading the library. A record built by
a function and handed to a parameter comes apart field by field, and
the gatherer that took it apart read only the record's own
declarations. The Spice3 records are three `extends` deep and nearly
everything they hold is declared above the name the value was written
against, so the value came apart into more pieces than there were
names to take them - and the hand-over was dropped whole, leaving
every field without a value. The same blind spot sat in four places:
the gatherer of names, the counter of numbers a record holds, the
walk that follows a field to its type, and the one that gathers a
record's starts. `record_components` already answered all four
correctly and is now what all four ask.

A red test was the judge of the second attempt, as the note on
performance keys says it usually is: a record's `constant` field
belongs to the class and not to any value of it, so counting it makes
the record one thing larger than the declaration and the value matches
neither reading. The battery records inherit a `constant String
CellType`, and the test that named them was written two shifts before
this change existed.

The second link is the language's own rule, and worth stating because
the refusal looked like a missing value: a variable of a function body
starts at its `start` attribute, and a field of an output record is
such a variable. `jfetInitEquations` assigns four fields of a record
of fifteen and leaves the rest at the zeros their declarations name.
Answering a generic zero for those would have been a guess - a red
test caught that too, because `SI.Voltage m_vds(start = -2.0)` starts
at minus two and what the declaration writes beats the default for the
type.

The third: `out_c := in_c` is how every Spice3 precalculation begins,
copy what was handed in and then write over the handful of fields this
step works out. The name on the right was bound field by field and so
had no value of its own to substitute, which left the copy answering
nothing. Copied field by field it is the statement the writer meant.

The chain was taken whole and moves no count: 820 flatten and 466 run
before and after, with the run list identical model for model. What it
moves is the wall - the twenty now refuse about singular structure and
underdetermined loops, real questions about the circuits rather than
about the compiler's reading of a record. `Inverter` reaches `no
equation determines vin.T0`, `CascodeCircuit` an underdetermined loop
in `J2.irs`. That is a family taken a storey up, and the run count is
a separate claim this change does not make.

## The census instrument was verified against itself

The shift that taught the census to keep its raw report also took its
measurements with it, which the charter forbids: a pure move must
leave the register identical line for line, and the register cannot
testify about a change to the register made in the same shift. The
check costs one pass and was owed. The script from before the change
and the script from after were run on one binary over the whole
corpus, and the two count sections diff clean at 276 lines. The only
difference in the whole output is the new line naming where the raw
report was written. Every number taken since is sound.

## `parameter has no value` was two layers, and one was ours

The row counted thirty after the Spice3 records were repaired, and
reading it as one family would have been a mistake in the other
direction from the usual: not a family split across wordings, but a
row holding two unrelated things.

Twenty of the thirty are utility and base classes - `OpAmpCircuits.PI`,
`Noise.Utilities.NormalDensity`, `ControlledDCDrives.Utilities.IdealDcDc`
and seventeen like them. Their parameters are unbound on purpose,
because the class is meant to be extended or instantiated with values
supplied, never simulated alone. The refusal is correct and the models
are not examples in any useful sense; what they are is the example
filter catching helpers, the same blind spot the charter names about
`--list` returning more lines than there are examples. Work aimed at
this row should count twenty-five as five.

The ten that were real models were one layer, and a nine-line model
showed it in a second:

```modelica
model InitParam
  parameter Real Av(fixed = false);
  parameter Real m_nom = 4.0;
  Real x;
initial equation
  m_nom = 2 * Av;
equation
  der(x) = -x + Av;
end InitParam;
```

The machinery to solve such a parameter beside the states already
existed and was reached through `unsettled_parameters`. What refused
the model was the step before it, which reads a `fixed = false`
parameter's `start` to have a guess for Newton and refused where none
was written. A start is a guess here by construction - `fixed = false`
says the declaration is not the source of the value - so its absence
is the zero the declaration would have given, not a missing value.

Five models left the wall: `HeatingSystem`, `Fourbar_analytic`,
`PlanarLoops_analytic` from the row itself, and `Engine1b_analytic`
and `EngineV6_analytic` from a row the counter worded differently. The
count of the row moved by three while five models moved, which is the
charter's warning about adding rows of one meaning before reading a
number, seen from the measuring end rather than the planning one.

## The unbalanced hundred is five families, not one

Split by what the refusal says nothing determines, the largest row of
the run half comes apart along physical lines rather than staying
whole: 25 machine-shaped (`airGap`, `friction.flange`, `psi_m`, with
`DCPM_Cooling` as a representative), 24 fluid (`heatTransfer.Ts`,
`m_flow`, `Wb_flow`; `AST_BatchPlant.Test.OneTank`), 21 circuit
(`opAmp` and bare pin currents; `CauerLowPassOPV`), 6 multibody
(`frame_a`, `frame_b`; `ForceAndTorque`) and 24 that match none of
these. By chapter the same models are 29 Magnetic, 27 Electrical, 22
Fluid across the two libraries, 10 Mechanics, 6 Media, 5 StateGraph.
No single repair reaches a quarter of the row, and the earlier note
that the machines and the row's top are different families holds in
the larger shape too.

## `cannot evaluate parameters` is five subfamilies, and the largest was a subscript

Read as one row, the top of the run half at 60 lines looked like one
wall. Sorted by what the refusal says nothing gives a value to, it
came apart into five: 23 whose missing name is a pipe's `dxs`, 15 that
name no variable at all but a function the compiler could not work out
(`dgesv`, `waterBaseProp_ph`, a colour map), 12 whose missing names
are the fields of a medium's `data` record, 3 on a multibody joint's
`rod1`, 2 on a pump's operating point, and one apiece besides. The
largest of them was bigger than the largest subfamily of the
unbalanced hundred, which is why it was taken first.

And it was not a family at all in the end: it was one rule with a hole
in it, in two links.

The first link. A value handed down an `extends` is written in the
names of the class handing it, and the base has never heard of them,
so the value comes back whole and is spread over every element. A bare
name was already taken apart one element apiece - that is what
`handed_shapes` is for. A name inside arithmetic was not: a pipe
writes `extends PartialTwoPortFlow(final dheights = height_ab*dxs)`,
and every element of `dheights` was bound to `height_ab` times the
entire array. Nothing can work that out, and the refusal named `dxs`,
which is a name the flat model does not have.

The second link, which the first stands on. Subscripting needs the
array's length, and `dxs` is declared `Real[n]` where `n` is settled
by the very `extends` carrying the modifier, two levels below the
declaration asking for it. Measured in source order, base first, the
length was simply absent. The reading now falls back to the numbers
the class settles for its own scalar names - but last, after every
cheaper reading, and with what the model handed in outranking the
class's defaults. That ordering is the whole of it: a first attempt
read the default `nNodes = 2` where the site had written `nNodes = 1`,
which is a wrong shape settled for good, and the charter's rule that
a missing shape is asked again while a wrong one is not is what
caught it before it was measured.

Taken as one series, since a link removed from the middle of a chain
moves no number by construction. The measurement: the `dxs` half of
the row fell from 23 to zero, and seventeen models moved, each now
standing at `unbalanced model`, one storey up. Seventeen and not
eighteen: the count of 60 lines falling to 42 was a count of
occurrences, and one of the eighteen it caught was the ranking
summary's own line at the foot of the raw report, which vanished with
the family it summarised. The named rows are the models: `cannot
evaluate parameters` 56 to 39 against `unbalanced model` 101 to 118,
seventeen either way. An occurrence is not a model, exactly as a row
is not a family. The census is otherwise unmoved by row, though not
quite identical line for line: `BranchingPipes15`, `16`, `17` and
`DynamicPipeInitialization` changed their wording without leaving
their row, `dxs` giving way to the next name nothing determines. The
five floors did not move: 2671 / 820 / 466 / 722 / 437. That is the
shape the charter describes for a change that moves a wall rather
than the count, and the wall behind this one is the unbalanced row,
whose fluid quarter these seventeen now join.

What is left of the row after the chain: the function-valued half (15)
is the largest remaining subfamily and is a different question
entirely - it asks the compiler to work out a call before the run, not
to subscript a name - and the medium's `data` fields (12) behind it.

## The unbalanced row read by victim rather than by wording

The row the seventeen joined is now the top of the register at 118
named lines, and read as one row it is not one family. Sorted by what
the refusal says nothing determines, the whole of it:

```text
35  fluid: a medium state inside heatTransfer or flowModel
28  machine and magnetic windings
27  everything else, mostly one apiece
19  electrical: a pin current or potential
 6  fluid: the m_flow of a device
 3  blocks: a connector nothing was joined to
```

So the largest true family in the register is the fluid one at 35,
which is larger than the 25 machine models the earlier reading called
the top, and larger than either of the two tops of `cannot evaluate
parameters` - the call before the run (16) and the medium's `data`
fields (16), both of which the seventeen also fed. Twenty-nine of the
thirty-five name `heatTransfer.Ts`, and twenty-eight name a field of
`heatTransfer.states`; they come from `ModelicaTest.Fluid` (23),
`ModelicaTest.Media` (6) and `Modelica.Fluid` (6).

## What a state record is worth when its fields arrive from below

The probe on the smallest of the thirty-five,
`ModelicaTest.Fluid.TestComponents.Vessels.TestSimpleTank`, says the
declaration is there and the equation between it and the medium is
not:

```text
declared: Real tank.heatTransfer.states[1].p
  bound to: nothing
named by: no equation of the flat model
```

The vessel writes `HeatTransfer heatTransfer(final states =
{medium.state})`, so there is a value; it is simply lost. Shrinking
the complaining model rather than growing a synthetic one puts the
cause in the shape of the record. `PartialMedium` declares
`ThermodynamicState` with no fields at all, and every medium fills it
by `redeclare record extends ThermodynamicState`. The table of which
instances are records is built by `collect_records`, which resolves a
declaration's type plainly - and plainly, `Medium.ThermodynamicState`
is the interface's empty one. Filed under a record of no fields, an
equation between two states is an equation between two empty lists,
and `push_equations` drops it without a word: the fields are declared,
so `why` finds them, and nothing names them.

The small model reproduces it exactly. With the fields written in the
base, the equation appears and the model settles; with the base left
empty and the fields moved into a `redeclare record extends`, the
refusal is the corpus's own, `nothing determines states[1].p`. That
is the mechanism, and it is one `AskedAs` mark short of the answer
instantiation already has - `record_asked_under` is what the component
layer calls for the same reason one layer down.

The chain is deeper than the mark, and the mark is worth its own
measurement. Set in `collect_records`, on the declaration and on the
walk below it, it moves the small model and does not move the vessel:
the vessel reaches its medium through a `replaceable package Medium`
redeclared at the instance, and the table is built with the imports of
the class rather than with the redeclarations that reached it - which
`collect_records` is not given at all. Measured over the corpus from
one baseline taken before it, the mark is worth a model: 820 flatten
to 821, the runnable pair 722 to 723, and the run counts stand exactly
where they were at 466 and 437. `ModelicaTest.Media.TestOnly.IdealGasN2`
crossed from `function SingleGasNasa.spe...` to flattening, and stands
now at `unknown variable medium.data.MM`, one wall further along. The
list of models that ran is identical line for line, so nothing was
paid for it.

What is left is the fifth link, and it is parked honestly with its
map: the family is the 35 fluid states, the layer is
`scoping::collect_records`, the missing piece is the redeclaration
environment that `settle_naming` works out one call earlier in
`instantiate` and does not hand on, and the test that goes red without
the part already taken is the shrunk vessel with its state filled from
below. Threading the environment through is a change to what the table
means for every model holding a record, not only for the fluid ones,
and it wants a run list of its own on either side.

## The fifth link taken, measured, and parked one link further on

The redeclaration environment does reach `collect_records`, and the
mechanism the previous shift mapped was right in every particular. A
component's `redeclare package Medium = Medium` is written in the terms
of the class holding it, so resolving the target there and putting it
in front of the entries the class below brought with it is what says
which medium a name means. The same has to happen on the base step:
`states` is declared as `Medium.ThermodynamicState` two `extends`
below the vessel that named the medium, and a walk that steps into a
base under the base's own names alone lands on the interface's empty
record again.

Shrunk from the real library rather than grown synthetically, the
witness is a vessel whose `Medium` alias sits in a base of the base -
`PartialLumpedVolume` holds `medium`, `PartialLumpedVessel` extends it
and holds the heat transfer model, and the heat transfer model reaches
`PartialHeatTransfer` through `PartialVesselHeatTransfer`. Three
earlier attempts at the small model settled without the change,
because with the alias written on the vessel itself the walk never
had to carry anything. The model that reproduces has the alias one
storey down from the component, and switching the change off gives
the corpus's own words: `nothing determines
tank.heatTransfer.states[1].p`.

Measured from one binary with the change behind
`OXIDELICA_RECORD_REDECLARES_OFF`, the two halves are these:

```text
off: 1043 examples, 821 flatten, 466 run; runnable 912, 723 flatten, 437 run
on:  1043 examples, 799 flatten, 466 run; runnable 912, 701 flatten, 437 run
```

The run counts do not move and the list of models that ran is
identical. The flatten count falls by twenty-two, and the diff of the
flattened lists names the victims: one gained,
`ModelicaTest.Fluid.TestComponents.Vessels.TestInitialization`, and
twenty-three lost, all of them pipes -
`ModelicaTest.Fluid.TestPipesAndValves.{Branching,Series}Pipes*`,
`ModelicaTest.Fluid.TestComponents.Pipes.DynamicPipe*`,
`Modelica.Fluid.Examples.{HeatingSystem,NonCircularPipes}`.

The victims are not a scattered loss. They are the same family one
link along, and the register says so: the row `an equation between
shapes [1, 5] and [1]` is absent before the change and carries twelve
lines after it, with sibling rows for `[1, 2]` and `[10, 5]`. The
model that stands for all of them refuses on

```text
an equation between shapes [1, 5] and [1]:
  Ref("pipe1.statesFM[2].phase") = Ref("pipe1.mediums[1].state")
```

which is exactly the wall the change was built to remove, seen from
the other side: `statesFM` is now filed as the medium's five-field
state and `mediums[i].state` is still filed as the interface's empty
one, so the two sides of an equation the pipe writes no longer agree
on what a state is. Making the component step carry the names in force
exactly as the base step does - the obvious symmetry, and it is a
fault worth naming, since the two steps had drifted apart - was
measured on a second full pass and gives the same 799 flatten to the
digit, with the flattened list identical line for line to the first.
So the asymmetry is real and is not what holds the pipes.

What holds them is the sixth link, and the shape of it is now known:
the record table is built per class as the walk goes down, and
`Modelica.Fluid.Interfaces.PartialDistributedVolume` declares
`mediums` as `Medium.BaseProperties[n]` in a class the pipe reaches
through a chain of its own, one the vessel's chain does not pass
through. The two names are filed by two different descents, and until
both descents carry the same medium the equation between them cannot
balance. The measurement to make first is which descent files
`mediums` and under what, since the fix is either a third place to
carry the names or - more likely, and worth asking before building -
one place for all three, which the drift between the base step and
the component step already argues for.

The work is parked honestly. The change is reverted rather than
committed: it costs twenty-two models and buys one, and a link taken
from the middle of a chain that has not been walked to its end is
exactly what the rules say not to keep. What the shift bought is the
map - the mechanism confirmed, the small model that reproduces it from
the real library, the two numbers from one binary, the victims named,
and the next link located rather than guessed at. The chain is now six
links deep, which is past the depth at which the map goes to a second
reader.

## The chain walked to its end: three links, two models, nothing lost

The map the previous shift parked was right, and the sixth link was
where it said. Walked with the probe rather than reasoned about, the
chain turned out to be three links and not one, and only the three
together move a number - which is exactly the shape the rules warn
about, where a link taken from the middle measures zero by
construction.

The fifth link is the redeclaration environment reaching
`collect_records` on both steps, the base one and the component one.
The sixth is what the probe was pointed at: a printed record table,
name by name, for the class that writes the equation the pipes refuse
on. It showed the table filled by three different descents:

```text
class=Q.PartialTwoPortFlow prefix=pipe1.            pipe1.statesFM -> Water.ThermodynamicState
class=Q.PartialMedium.BaseProperties prefix=pipe1.mediums[1].
                                          pipe1.mediums[1].state -> Water.ThermodynamicState
```

Both entries are right, and they are in different tables. The walk
that descends through each element of an array files
`mediums[1].state` under a prefix that already carries the subscript;
the class writing `statesFM[1] = mediums[1].state` never steps
through a subscript itself, so its own table holds one side of that
equation and not the other. One side is then written out as the
medium's five fields and the other stays a bare name, which is the
`an equation between shapes [1, 5] and [1]` the whole family refused
on.

The seventh link is smaller and was found by taking the sixth: the
lookup shortens a path by cutting subscripts off from the right
_before_ asking the table about the path as written. A name the walk
filed with its subscripts on is thrown away by the shortening that
was meant to find it. Asking as written first, and shortening only
after, is the whole of it.

Measured from one binary, the whole chain against nothing:

```text
off: 1043 examples, 821 flatten, 466 run; runnable 912, 723 flatten, 437 run
on:  1043 examples, 823 flatten, 466 run; runnable 912, 725 flatten, 437 run
```

The run list is identical line for line and the flattened list gains
two without losing any:
`Modelica.Fluid.Examples.HeatExchanger.HeatExchangerSimulation` and
`ModelicaTest.Fluid.TestComponents.Vessels.TestInitialization`.

The sixth link was built twice, and the first build is worth
recording. Handing the class the accumulated table whole cost the two
media models `ExtendedProperties` and `TestTwoPhaseStates` of
`Modelica.Media.Examples.TwoPhaseWater`, on `dynamicViscosity wants 5
field(s) for state, got 0`: an entry another descent had filed under
the interface's empty record outranked the one the class had resolved
for itself. Narrowed to the names a
class genuinely cannot reach - those below a subscript, under its own
prefix - both came back and both gains stayed. The narrowing is by a
test on the path, which is a guess of the kind this repository
distrusts, and that is what the consultation now standing in
`QUESTION_FOR_FABLE.md` asks about: whether three descents filling one
table should be one place instead.

What the chain does not buy is a running pipe. `HeatingSystem` and its
family now flatten and stop one wall further on, at `cannot evaluate
parameters [tank.h_start = ...]`, which is a different family with a
map of its own. So this is a count that moved by two and a wall that
moved for a family of twenty-odd - the second being the larger half,
and the register is where it shows.

## What the second reader said, and where the next shift starts

The consultation was asked and answered in the same shift, so the map
for what follows is written down rather than guessed at. It took the
narrowing by `"]."` not as evidence that the place is wrong, but as
evidence that the table does not remember which descent wrote an
entry, so the provenance had to be encoded in a substring of a name.
The repair it argues for is not to teach the table to remember the
writer, but to leave it with one.

The place is not a new one. `acc.instances`, `acc.connectors`,
`acc.handles` and `acc.sizes` are all written by the builder at the
point of building; the record table alone is filled by a
reconstruction, and three times over. The insertion it names is at the
component descent in `components.rs`, where `child` has already been
through `record_asked_under` and the flat name already carries the
subscript - one line, no reconstruction, and the `AskedAs` chain still
standing because real calls hold it.

Two warnings come with it and both are worth keeping. The symmetric
place, beside `acc.instances` in `instantiate`, is a trap:
`instantiate_bases` re-enters with the same prefix, so a `record
extends` would have the base's pass overwrite the derived record with
the interface's name - the fifth link's own disease, reproduced in one
line. And a pass over the finished flat model is both too late, since
equations are flattened in the middle of the walk, and blind, since
the flat model has already dissolved a record into its fields.

On the third question it splits the answer: shortening a path by its
subscripts is a definition against a key that names a declaration, and
a guess against a key that names an instance. Both kinds share one
table today, which is why the shortening looks like guesswork; the
seventh link put the exact key first, and the reform would make that
precedence structural rather than incidental. The loop is not to be
removed - `expand` serves both phases, and during building the table
it reads is the declarations' one, where shortening is lawful.

It also left predictions to be caught by, which is the half that makes
the next shift cheap: the corpus under the reform moves only models
holding a modifier that names a record under a neighbour not yet
built; `HeatingSystem` does not move under any step of it, since
`cannot evaluate parameters` belongs to `settle_parameters`, which
reads no records at all; and the shortening loop, instrumented with a
counter, falls silent during the equation phase. Anything else that
moves is a burden on the one-level table that the answer did not see.

## The medium's reference constants: a chain walked to six links and parked

The census at `fb5df36` puts `cannot evaluate parameters` at the top of
the run half once the wordings are added together: 42 models refuse
there, against 29 for the largest algebraic-loop row. The family is the
fluid one, and `Modelica.Fluid.Examples.HeatingSystem` is its
representative - it flattens and then refuses with `nothing gives a
value to beta_const, cp_const, kappa_const, reference_d, reference_h`.

The reader's prediction from the previous shift is confirmed by
measurement: `HeatingSystem` does not move under any step of the record
table reform, and the wall it stands at belongs elsewhere. But the
report's guess at where was wrong. It is not `settle_parameters`; it is
the constants layer, and the mechanism is exactly the breed AGENTS.md
now counts for the third time - two spellings of one name taking two
different roads.

`Modelica.Media.CompressibleLiquids.LinearWater_pT_Ambient` extends a
base that gives its reference constants by calls into the IF97 steam
tables: `reference_d = StandardWater.density(state)`, where `state` is
itself `constant ThermodynamicState state = setState_pT(reference_p,
reference_T)`. Written with a path, `Medium.reference_d` is answered by
`class_constant_binding_at`, which hands the call on for the run to
walk. Written bare - which is how a body of the interface says it, and
how it arrives after inlining - nothing hands anything on, so the name
travels into the run and the parameters cannot be evaluated.

The chain was walked with a probe, each removal local and unrecorded,
and it does not end inside a shift. The links, in order:

1. A bare constant under the asked-as medium has no road handing on a
   call binding. Built: `asked_as_constant_binding`, the twin of the
   dotted road. Fires; `reference_d` now comes back as `state.d`.
2. `record_constant_field` reads a record constant's fields off its
   modifier list only. A record whose whole value is a call is not
   read at all. Built: take the field by its declared position out of
   the array the record layer answers a record-returning call with.
3. That road never reaches, because `state` is bare too, and
   `record_constant_field` splits on a dot and gives up without one.
   Built: `asked_as_record_owner`, the mark again.
4. The component is not found even then, because the lookup reads
   `owner.components` and `state` is declared by the base the medium
   extends. Built: `with_inherited_components`.
5. The call now stands with its arguments still bare - `setState_pT(
reference_p, reference_T)` - because they too are given values by
   the medium's `extends`. Folding them from `gathering_settled` works
   and costs the shift's second scar: one model went from eleven
   seconds to over six minutes, since asking the gathering settles the
   whole IF97 basket for a binding that mostly needs none of it.
   Ordering the cheap substitution first and the gathering only on
   what it could not answer brings it back to nine seconds.
6. And there the walk stops without reaching the end. With arguments
   folded to numbers the call is still not inlined -
   `PartialTwoPhaseMedium.setState_pT` is a `redeclare replaceable`
   whose body calls `setState_pTX`, and what comes back is an array of
   two where a scalar is wanted. `HeatingSystem` no longer flattens at
   all, which is worse than the wall it started at.

So the chain is parked, not taken: six links deep, the sixth not
mapped, and the state at the end of five is a regression rather than a
gain. The work is parked honestly - the code is reverted and the
baseline is the same `cannot evaluate parameters` as before - and the
map above is what a later shift starts from rather than rediscovering.

Two things worth carrying out of it regardless of the chain. The
minting ledger's price ordering has a third instance now, and it is
written into AGENTS.md's performance note by implication rather than by
name: the dear reader is `gathering_settled`, and it must never be the
first test. And the probe that made this cheap was the same one each
time - print what a road hands on, run `--only` on the one model, five
seconds a round. Four corpus runs would have said less.

### Link three paid, and a fourth instance of the same scar

The chain's third link is now taken: a field of a record constant named
without a path is read. A package writes `constant St ref(p = 1e5)` and
then `constant Real d = ref.p` beside it, and the reader took the head
apart on a dot, so a sibling record was unreachable and every field of
one was left standing as a bare name. Two tests hold it, one for a
record the package declares and one for a record a base declares, and
the second was red until the lookup was taken through the `extends`.

What that lookup cost is the finding, and it is the fourth appearance of
the ordering scar in this one layer. Reaching through the `extends`
means gathering every component of every base, which allocates, and the
first version asked for the gathering before asking whether the class
declared the name itself. That question is put to every dotted name in a
library whose head is not a class, which is most names that are not
classes, so the gathering ran unconditionally. One model measured it:
`Modelica.Media.Examples.MoistAir` went from two seconds to eighty-eight,
and the whole corpus from eleven minutes to over two hours, standing for
a third of that on a single model. Asking the free question first -
the class's own components - and the dear one only of a class that
extends something brings MoistAir back to two seconds and the corpus to
eight and a half minutes, with the same five floors and a ran list
identical name for name.

The instrument that settled it was `sample` on one `--only` run, not the
corpus. The profile named `class_constant_at` calling
`function_components` outright, which is the whole diagnosis in one
line; the two corpus runs the previous shift spent on the same question
said only that something was slow. A giant is found with a narrow pipe
and a profiler, and the corpus is what confirms the cure afterwards.

The wall behind the link is named, and it is not where the chain was
left. `Modelica.Fluid.Examples.HeatingSystem` still refuses at
`cannot evaluate parameters`, but `tank.h_start` now reads as a mix of
prefixed and bare names - `reference_h`, `cp_const`, `reference_d`
carrying no path beside `heater.reference_h` that does - which is the
flat model's own invariant broken: anything that survives flattening
carries the flat model's names. The next shift starts there rather than
at link six.

### Links four and five, and what a refusal was letting go

The mixed names of the previous note had one writer, and it was the
mint that turns a medium's constant into a name of the flat model.
The mint asks for a number before it writes the name, and where there
is none it refuses - and what the refusal let go was the _bare_ name,
the one the body was written with. `reference_h` of a linear water
medium is bound to a call on the steam tables that no arithmetic
folds, so the mint said no and the name arrived in the flat model with
its prefix gone, beside `heater.reference_h` on the other branch of
the same `if`, which had a number and so was written properly. That is
the invariant broken from the inside: a name shortened to its tail is
a guess, and here the guess was made by the one road that knew the
full name and chose not to write it. The binding is handed on instead,
read under the medium on the mark, which is what the dotted road
already does in the same spot.

The link below it is the record the reference constants are read from.
`constant ThermodynamicState state = setState_pT(reference_p,
reference_T)` is declared empty in the interface and given its value by
the `extends` of whichever medium the model chose, and the record road
walked outwards to the interface, found the declaration, and answered
with the blank. The blank looks exactly like an answer - the same "one
text, two gatherings" the scalar road already guards against - so the
medium on the mark is asked first, and the value is taken from the
`extends` through the gathering rather than from the declaration.

Both links cost a model, and it is the model the work was aimed at.
`Modelica.Fluid.Examples.HeatingSystem` used to flatten while carrying
a name nothing declared, which it could never have run on; it now
refuses one wall further along, at an array in an equation rather than
in a parameter. That is a model given back for a fault removed, and
the floors move down with it - 823 to 822 flattened, 725 to 724
runnable - because a ratchet that is not wound is not a ratchet, and
one wound in the wrong direction knowingly is better than one wound on
a number nobody measured. The run halves did not move: 466 and 437,
and the diff of the two ran lists is empty.

The chain has a fifth link and it was walked before the shift closed.
A record constant handed to a call as a whole - `reference_d =
density(state)` - reaches the same mint, and there the gate that
stops it is the unit: a constant with no unit is not a candidate, by
design, because a name buys nothing over a digit without one. Where
the declaration does carry a unit the chain runs clean through, which
is what the two tests show. What stands behind link five is the record
builder, and that is parked ground rather than this chain's.

## A port of the top model has nothing above it

The census taken at `e7a563b` put `unbalanced model` at the head of the
run half by a wide margin: 91 models once the rows the counter had
split by their wording were added back together, against 43 for
`structurally singular model` and 39 for `cannot evaluate parameters`.
Probed rather than read, its top entry was not one family but it held
one that was plain: seven models of the operational amplifiers, each
short between two and nineteen equations, each naming its own ports -
`n2.i`, `p1_2.i` - among the unknowns nothing determined.

The mechanism is the seam, seen from the end the compiler had not
looked at. A port is joined from two sides, and the flat model owes an
equation for each; where the level above leaves a submodel's port
alone, the compiler already mints the zero that says nothing outside
takes the flow. The model being flattened has no level above it at
all - it is the whole of the run - so its own ports are joined from
inside by its own `connect` and from outside by nobody, and the zero
they were owed was never written. A circuit written as a reusable
block with four pins of its own is short exactly four equations, one
per pin, and that is what the count said.

What nearly hid it is the shape of the test that withholds the zero,
and it took two goes to get right. A port the model speaks for must
not be given a second value, and asked as "is the port named
anywhere", the four-pin interface of the electrical library answers
yes for every port it has - `i1 = p1.i` introduces `i1` and determines
`i1`, not the port. Naming is not speaking for. Narrowed the other way,
to only the flow standing on the left, the test let through a ground
written as a port of the top model, `g.v = 0`, where the current the
ground draws is exactly what the connection sum is there to find; that
cost three simulation tests their balance, and the preflight caught
it. What silences a port is any member of it standing alone on the
left of an equation, which is both readings' answer where they agree
and the right one where they do not.

Measured with one binary and an environment switch, twice over the
whole corpus, and the settled rule measured again against the same
baseline for the same numbers: 466 models ran without the seam and 471 with it, 437
runnable against 440, and the flattened lists are identical line for
line. The five are `CauerLowPassOPV`, `CauerLowPassSC`,
`DifferentialAmplifier`, `SwitchedCapacitor` and
`TranslatoryArmatureAndStopper` - the last of which was parked ground,
freed by a fix aimed elsewhere. Nothing was lost. The rest of the
opAmp family did not come with them: their balance now closes and they
refuse one storey up, as `structurally singular model: equation
Ref("p1.i") = Number(0.0) constrains no state`, which is a wall of its
own and the next thing to probe.

## A ceiling that fired on noise, and how it was told apart

Two time ceilings fired within a day of each other, and both read as a
regression until they were measured. CI called `e7a563b` red with
`flattening is 7023ms per model, and the ceiling is 7000ms` - a third
of a percent over - against 4178ms and 4456ms on the two commits
before it, on the same machine over the same 1043 models. A jump of
58% in one commit, and the commit in question touched the constants
layer, which is the layer the notes above say has cost a whole shift
three separate times. The reading wrote itself.

It was wrong, and what showed it was two binaries against a handful of
models rather than a fourth corpus pass. `5444b45` built into its own
worktree, `e7a563b` built beside it, six models timed one by one from
the root of the corpus - the fluid models where the medium's constants
are in play, and `ChuaCircuit` and `DoublePendulum` as controls:

```text
                        5444b45   e7a563b
HeatingSystem             12.14      6.19
PumpingSystem              4.93      4.95
HeatExchangerSimulation   10.40     10.38
ThreeTanks                 1.32      1.32
DoublePendulum            31.62     31.90
ChuaCircuit                0.31      0.31
```

The change made the model it was aimed at twice as fast and left
everything else alone to the digit. Nothing in the fluid family got
dearer, which is where a slowdown from that commit would have had to
live.

What the number was is desk noise of the same size. Two whole-corpus
runs from that shift, both on the same code and both reporting the
same 466 models run, printed 1725ms and 3412ms per model - a factor of
two between two runs of one binary. A machine that can vary by 98%
between identical runs can certainly clear a ceiling by 0.3%, and the
`48461ms` in a third log from the same night is the same instrument
catching a laptop that went to sleep mid-pass.

So the rule this leaves: a time ceiling firing is a question, not a
verdict, and the cheap way to answer it is two binaries over six
models rather than another pass over a thousand. A regression in a
layer shows up as a family getting dearer while the controls hold
still; noise shows up as everything moving together, or as one number
moving and no model behind it. The ceilings stay where they are - one
firing that was measured and explained is the instrument working, and
raising it on the first fire would be turning it off.

## What became of the opAmp family

The seam above was aimed at the operational amplifiers, and the note
that reported it said the rest of the family would refuse one storey
up as `structurally singular`. Probed with the seam in place, that
turns out to overstate what is left. Of the fourteen examples under
`Analog.Examples.OpAmps`, thirteen now flatten and run: Adder,
Comparator, Differentiator, HighPass, Integrator, InvertingAmplifier,
InvertingSchmittTrigger, LCOscillator, LowPass, Multivibrator,
NonInvertingAmplifier, SchmittTrigger, SignalGenerator, Subtracter and
VoltageFollower among them.

One does not: `ControlCircuit`, which flattens and then refuses with
`initialization is not square: 3 initial equation(s) and 4 fixed`.
That is not the family's own wall but the parked triple the notes
already carry, met by a fourteenth model. So the opAmp family is not
work waiting at the top of the census; it is finished except for a
model whose remaining barrier belongs to another queue. Whoever reads
the unbalanced row next should subtract the opAmps from it before
planning against the number.

## The measurement that settled the ceiling

The note above told the two firings apart by building two binaries and
timing six models, and concluded noise. A cleaner witness arrived
afterwards and is worth recording, because it needs no second binary
at all: `e7a563b` was run twice by CI on the same machine, once by the
push and once by the scheduled pass an hour later. Same commit, same
1043 models, same runner image.

```text
push      7686ms per model flattening   red
schedule  5939ms per model flattening   green
```

A spread of 29% on identical code, with nothing between the two runs
but which hardware the day handed out. The ceiling stood at 7000, in
the middle of that band, so which side of it a commit landed on was a
coin toss - and a threshold that fires on a coin toss teaches its
readers to ignore it, which is worse than not having one.

That is the third outcome the shift was told to look for, and it is
the one the numbers support: not noise to be waved away and not a
regression to be fixed, but a boundary drawn too tight to be read.
The ceilings are therefore raised, to 12000 and 8000, with the spread
that justifies them written beside them in the script. They still
catch what they were built for - the index reduction that went from
591s to 3153s is a factor of five and clears them by a mile - and they
no longer fire on the weather.

## The machine family, and the refusal that named no equation

The machine models were taken up as the largest family inside the
unbalanced row: a dozen or so entries written in one hand, `aimc`,
`smee`, `smpm`, `smr`, each saying `nothing is left for` some flange
angle. The first question was whether they are one layer or one
wording, and probing answered it in one second apiece, from the root
of the corpus rather than from a subtree.

They are two libraries with similar names, not one family. The
`Electrical.Machines` examples - `IMC_DOL`, `IMC_YD`, `IMC_Inverter` -
do not refuse as unbalanced at all; they reach a `residual N of
algebraic loop` or an `initialization is not square`. It is the
`Magnetic.FundamentalWave` machines that are unbalanced, and there by
exactly two equations: 813 for 811, 830 for 828, 811 for 809. The
same excess of two across a dozen models is the signature of one
cause, but it is not the cause the census's wording suggested.

Probing the balance of `Magnetic.FundamentalWave...IMC_Inverter`
printed eight equations with nothing left to solve for, and one of
them was this:

```text
balance: nothing is left for its limit = 0
```

`its limit` is not an equation. It is the phrase a bound's assertion
uses for a right-hand side that is neither a number nor a name, and
the unbalanced refusal had borrowed that spelling to print equations
with. Any equation whose side is a sum, a call or an index came out
as those two words, so a refusal built to name what the model is
short of named nothing at all - the one thing this compiler's notes
say a refusal may not do. The parser's own `Expr::describe` writes
the expression out, and the refusal now uses it. The test is a
two-equation model whose extra equation is `y + y = 2`: before the
change it refused with `nothing is left for its limit = 2`, which is
the library's exact wording in twelve characters.

That is a repair to an instrument rather than to the compiler, and
it must not be credited with anything else. The census taken this
shift ran on the binary as it stood before the change, so the drop it
shows in the unbalanced row - 91 to 63 against the previous census -
belongs to the commits between the two and not to this one. What
moved with it is instructive on its own: the rows naming algebraic
loops went 47 to 55 and the structurally singular rows 43 to 58,
while the run half's refusals in total fell only from 356 to 351. The
models walked from one wall to the next, as the notes predict of a
family taken one storey up, and a reader who had counted the
unbalanced row alone would have claimed twenty-eight models that were
never won.

What this change buys is that the next reader of that row sees which
equations are surplus instead of a phrase repeated eight times. It
costs no models and wins none, and the honest measurement of it is
the test that reproduces the library's wording in twelve characters.

## Seventy-two percent of a measurement spent on three models

The library check takes as long as its dearest models, and `--slow`
says which those are. Measured on a quiet machine over 1043 models,
the run half cost 3757s and three names took 2693s of it:

```text
1643s  Spice3BenchmarkFourBitBinaryAdder.FOURBIT
 825s  Spice3BenchmarkFourBitBinaryAdder
 225s  ...TWOBIT
```

Seventy-two percent of the time on three models of one package. The
flatten half is milder and the same shape: `EngineV6` at 241s and
three `BranchingPipes` at 145 to 177s take 925s of 3028.

That is what pushed the library job into its ninety minute ceiling,
where a cancelled run leaves the same blank space on the page as a
green one - two commits went without a verdict from it for exactly
this reason. And what those three models measure is the speed of the
numerical solver on a large circuit rather than how much of the
library this compiler reads; they grow dearer on their own as more of
them passes.

So they are carved out, and the carving is written down rather than
done quietly. `scripts/heavy_models.txt` names them with what each
cost and the date it was measured; `library check --without <file>`
takes them out of the main run and `--only-from <file>` makes them the
whole of a scheduled one. One file serves both, so the two cannot
drift apart.

The half that makes it honest is the second floor. A model taken out
of a measurement and given none has its regression hidden for ever, so
`scripts/heavy_floor.sh` holds the same counts over exactly this list,
and it checks the length of the list too: a name quietly deleted would
otherwise lower every count it holds and pass.

The floors of the main run came down by exactly what moved, and the
arithmetic is in the script so that nobody reads it as a regression a
week later: flatten 822 = 819 + 3, runnable flatten 724 = 723 + 1. The
run halves did not move at all, 472 = 472 + 0, because none of the
three runs yet - the time they cost is spent reaching a refusal, an
underdetermined algebraic loop in `FOURBIT`'s case. Which is the
finding worth keeping separate from the saving: the dearest model in
the corpus is dear on the way to failing.

A note on what was not carved out. `DynamicPipeEnergyConservationCheck`
costs 199s and stays, because what it measures is whether the answer is
right, and a physical check is not a benchmark however dear it is.

A slowdown was suspected in the compiler before this was measured, and
there was none. The same models timed one at a time on the current
binary came out at 1598s for `FOURBIT` against the 1643s measured
clean, and 196s for `EngineV6` against 241s. Both are inside the noise
band these notes already record for the corpus, in the faster
direction. The 91 minute CI runs and a 32 minute local pass were the
runner's weather and a stray parallel job, not a regression - which is
why nothing was fixed here, and why the numbers are written down: a
phantom chased twice is a shift lost twice.

## The machine family is two families, and the refusal shows one side

The thirteen machine models in the unbalanced row were treated as one
barrier because the names in their refusals rhyme - `airGap`, `pin_ap`,
`fire_n`. Probed with `OXIDELICA_BALANCE_PROBE=1`, they are two shapes
that have nothing in common but the library they live in.

`Modelica.Electrical.Machines.Examples.InductionMachines.IMC_withLosses`
is 501 equations for 504 unknowns: three equations short. The probe
names thirteen unmatched unknowns, and `oxidelica why` finds that every
one of them already has an equation of its own -
`aimc.powerBalance.powerMechanical` has exactly one, and so does
`aimc.airGap.spacePhasor_s.v_[1]`, and so does `combiTable1Ds.y[2]`,
which belongs to a table and not to a machine at all. So the thirteen
are the leftovers of a matching that ran out of equations, not the
cause of anything: what to look for is the three equations the flat
model never wrote, and no victim in the list points at them.

`Modelica.Magnetic.FundamentalWave.Examples.BasicMachines.InductionMachines.IMC_DOL`
is the mirror image: too many equations, and the probe names the
equations with nothing left to solve for. There the list is not
leftovers - it is the connection equations of a polyphase converter,
`singlePhaseElectroMagneticConverter[k].port_n` against
`[k+1].port_p`, a chain drawn inside a component, alongside ordinary
`flange.phi` equalities. The two halves of the family therefore need
opposite work: one is a missing definition, the other a connection
counted twice.

What the census cannot show, and this is the reason to write it down:
both print the word `unbalanced`, so they are adjacent rows of one
instrument and read as one queue. The probe separates them in a
second each, and the earlier reading - "their bus sits inside a
component, not at the top" - is true of the second shape only.

Parked rather than fixed, because neither shape has a small model yet.
Four small models were written against the first shape and all four
were red for a reason of their own making: an unconnected connector
already gets its flow equation, so a hand-written probe that leaves
one dangling refuses correctly and proves nothing. The layer is not
clear enough to change, and a definition-adding change measured
against an unclear layer is how nine models were lost before.

## The thirteen machines are eight, two and twenty-two

The family was taken by name from a census that was three commits old,
and the first thing the probe said was that the census was stale.
Eight of the thirty-two machine-shaped models in that unbalanced row
run now: `DCMachines.DCPM_Cooling`, the four
`PowerConverters.DCDC.HBridge` examples, and the three
`Magnetic.QuasiStatic.FluxTubes` airgap examples. Nothing in this
shift moved them; they were won earlier and the census had not been
retaken. That is the cost of choosing work from a file rather than
from a run, and it is cheap to avoid - `--only` answers for one model
in under a second, and thirty-two of them took three minutes.

Of the twenty-four that remain, two are short of equations and
twenty-two have too many. The shortage pair is
`Electrical.Machines.Examples.InductionMachines.IMC_withLosses` at
501 for 504 and its `FundamentalWave` namesake at 864 for 865. The
excess is uniformly two equations in nine of them -
`IMC_Conveyor`, `IMC_Initialize`, `IMC_Inverter`, `IMC_Steinmetz`,
`IMC_YD`, `SMEE_Rectifier`, `SMPM_Braking`, `SMPM_CurrentSource`,
`SMPM_VoltageSource`, each at exactly +2 - and larger in the thirteen
polyphase and comparison models, which carry the same converter
several times over.

### The probe's list is leftovers on both sides, not the cause

The earlier map read the excess side's list as the culprits, because
it names connection equations of the polyphase converter and the
shortage side's list reads as unremarkable leftovers. Measured, the
two sides are the same kind of list. `SMPM_Braking` shrunk to a
machine, a star and a resistive load is 552 equations for 551 - one
too many - and the probe prints eleven equations. `IMC_Conveyor` is
two too many and the probe prints eighteen. A list whose length has
no relation to the excess is what a matching leaves behind when it
runs out, not what it could not place. So the instrument answers
"which equations were not reached", and on neither side does that
name the equation that should not exist.

### The converter chain is not the shape, and the winding runs

The chain drawn inside `PolyphaseElectroMagneticConverter` -
`connect(singlePhaseElectroMagneticConverter[k-1].port_n, [k].port_p)`
in a `for` loop, with the ends on the component's own ports - was the
suspected shape, and two small models say it is not. A chain of that
drawing over a scalar `flow` runs; the same chain over a `flow` of
record type, which is what `Phi` is, runs too; and adding a
conditional array of components connected to every link, which is what
`strayPermeance ... if useStrayPermeance` is, still runs. The
suspected shape was also `Modelica.Mechanics`-shaped in
`PartialBasicMachine` - a protected `internalSupport` joined to a
`Fixed` that exists only when `useSupport` is false - and a small
model of that runs as well.

The real `SymmetricPolyphaseWinding`, driven from a polyphase source
into a reluctance, runs on its own with the library's own default
`ratioCommonLeakage = 1`. With the ratio at zero, so that the
conditional `stray` permeance is absent, it does not run - but the
refusal there is an algebraic loop residual, not an imbalance. So the
winding is not where the two equations are, and neither is the
converter beneath it.

`useSupport` makes no difference to the count either: `true` and
`false` both give 552 for 551 on the shrunk model. What does change it
is the phase count, and in the wrong direction - the same machine at
`m = 5` is 585 for 589, four equations _short_. A family whose sign
flips with a parameter that ought only to change its size is one fault
seen from two ends, and the excess and the shortage are very probably
the same thing.

Parked, and more honestly than last time. The barrier is above the
winding and below the example, in what the machine assembles from
`airGap`, `permanentMagnet`, `strayLoad` and the mechanical chain; a
model small enough to hold it has not been found, and four small
models of the shapes that were suspected are all green. The next
probe worth building is one that prints which equations a matching
_did_ place against the machine's own components, since the list of
what it could not reach has now been shown to say nothing on either
side.

## The excess is exactly two fewer than the phase count

The earlier reading said the sign of the imbalance flips with the phase
count: the shrunk machine at `m = 3` was one equation too many and at
`m = 5` four equations short. Swept properly it does neither. The
shrunk machine gives, from one binary and one file per point:

```text
m = 3   552 equations for 551 unknowns   excess  1
m = 4   675 for 673                      excess  2
m = 5   752 for 749                      excess  3
m = 6   852 for 848                      excess  4
m = 7   952 for 947                      excess  5
```

The excess is `m - 2`, exactly, with no kink at the even counts and no
sign change anywhere. There is no nonlinearity and no parity to explain
and no second cause: one equation too many per phase, less two.

The earlier numbers were an artefact of the measurement, and the fault
is worth naming because it is easy to repeat. The shrunk model declares
its own `constant Integer m` and passes it to the star and to the load,
but the machine's `m` lives in `FundamentalWave.BaseClasses.Machine` and
was never modified. So the sweep changed the phase count of everything
around the machine while the machine itself stayed at three, and the
mismatch between a five-phase load and a three-phase machine is what
produced the four-equation shortage that read as a sign flip. The probe
said so plainly and was not asked: at every `m` the unmatched list named
`singlePhaseElectroMagneticConverter[1..3]` and no more, while
`load.resistor[1..m]` grew as it should. A component that does not grow
with the parameter being swept is a parameter that was not passed.

With `smpm(m = m, ...)` added the converter array grows to `m` and the
law is the clean one above.

### Which layer carries the extra equation, by elimination

Each of these was built as a small model of its own and swept over the
same phase counts, from the same binary:

- the real `PolyphaseElectroMagneticConverter`, driven from a polyphase
  source into a reluctance: balanced at every `m`, refusing later for a
  singular structure rather than a count;
- the real `SymmetricPolyphaseWinding`, the same way: balanced at every
  `m`, and balanced again with `useHeatPort = true` and its conditional
  `heatPortWinding[m]` connected, which was the next suspect;
- a polyphase source, resistor and two stars: runs at every `m`;
- the machine's own `Losses.InductionMachines.StrayLoad` with its flange
  and support: runs at every `m`;
- a component of one's own whose polyphase plugs are joined straight to
  an inner component's, which is the shape `plug_sp`/`plug_sn` have:
  runs at every `m`.

So the per-phase excess is not in the converter, not in the winding, not
in the polyphase basics, not in the loss components, and not in the bare
plug-to-plug boundary. It is in what
`FundamentalWave.BaseClasses.Machine` assembles from them, and the
assignment probe locates it no further than that: swept across `m`, the
unknowns reached under `airGap` stay at 36, under `friction` at 9 and
under `powerBalance` at 11, while `stator`, `strayLoad`,
`thermalAmbient` and `permanentMagnet` grow, which is what they should
do. The thermal pair was swept on its own and is short by one at every
`m` - a constant, not a term in `m`, so not this family.

Parked, and the parking is the finding: the sign never flips, so the
shortage family and the excess family are not one fault seen from two
ends, and the note above saying they probably were is withdrawn. What
remains to find is a single equation written once per phase where one
should be written per machine, inside the base class's own `equation`
section or its `connect`s, and five green small models now say where it
is not.

### The probe now says what was placed, not only what was missed

`OXIDELICA_BALANCE_PROBE=1` printed the unknowns or equations a matching
could not reach, and that half has been measured and found to say
nothing: on both sides of an imbalance the list's length has no relation
to the excess, because what a matching leaves behind when it runs out is
not what it failed to place. It now also prints one `assigned <unknown>
<- <equation>` line per unknown it did reach, which is the half a model
can be read by - it is what shows that `airGap` holds 36 unknowns
whatever the phase count while `strayLoad` grows by six a phase, and
that is how the elimination above was done.

## The second excess is the shaft, and it is not in `m`

The shrunk machine's excess is `m - 2`, measured above. The full
`SMPM_Braking` is two too many rather than one, and the debt of the
previous shift was to say where the difference of one lives. Measured,
from one binary, by putting back one at a time what the shrinking threw
away:

```text
inertiaLoad on the shaft            558 for 556   excess 2
speedSensor on the shaft            555 for 553   excess 2
a Fixed on the shaft                555 for 553   excess 2
two inertias in a chain on it       563 for 561   excess 2
currentQuasiRMSSensor               607 for 606   excess 1
voltageQuasiRMSSensor               607 for 606   excess 1
terminalBox and a second load       625 for 624   excess 1
the diode bridge and its resistor   692 for 691   excess 1
grounding resistor                  561 for 560   excess 1
```

So the second equation is bought by joining anything at all to
`smpm.flange`, and by nothing else. It is one, whatever hangs there:
a `Fixed`, a sensor, an inertia, or two inertias in a chain all cost
exactly the same one. And it is constant in the phase count - swept at
three, five and seven the excess with a shaft connection is `m - 1`
against `m - 2` without, the same single equation at every point. Two
families, then, and the shift's other rule says not to mix them: the
per-phase one is the stator's, and this one is the shaft's.

### Which equation it is, and why the obvious repair is not one

The assignment probe names it outright. Without a shaft connection the
machine's own binding takes the flange angle:

```text
assigned smpm.flange.phi      <- smpm.phiMechanical = smpm.flange.phi - smpm.internalSupport.phi
assigned smpm.internalSupport.phi <- smpm.internalSupport.phi = smpm.airGap.support.phi
```

With one, the connection takes the angle and the binding is pushed down
onto the support, which leaves the support's own connection equation
with nothing to do:

```text
assigned smpm.flange.phi          <- smpm.flange.phi = fx.flange.phi
assigned smpm.internalSupport.phi <- smpm.phiMechanical = smpm.flange.phi - smpm.internalSupport.phi
nothing is left for                  smpm.internalSupport.phi = smpm.airGap.support.phi
```

`phiMechanical` is an output with a binding, so it is a definition and
not an equation to be solved - and the matching treats it as one more
equation naming two connector angles. That reading is confirmed from
the other end: replacing `tauShaft = -flange.tau` with a constant in a
scratch copy of the library takes the machine from 552 for 551 to 553
for 551, one further out, because the reading of `flange.tau` is what
tells the seam rule the port is spoken for. Both bindings are being
counted by rules that were written for equations.

A repair was built and measured and is not in the tree. A port joined
from both sides of its own class is cut into two connection sets, keyed
by the side each member was joined from, and each half is summed on its
own; merging the halves back into one set is the obvious architectural
fix. Behind `OXIDELICA_NO_SEAM_MERGE`, one binary, it moves nothing at
all: 552/551, 755/751 and 955/949 at three, five and seven phases with
the merge on and off alike. What it does is trade one equation for
another - the flow sum loses a term and a potential equality gains a
member - and the net is zero at every point measured. A change that
measures zero on six points is not a fix, so it was reverted rather
than kept for looking right.

The smallest model that holds the shape is twelve lines and needs no
library at all:

```modelica
model HK
  connector P
    Real e;
    flow Real f;
  end P;
  model Inner
    P port;
    P a;
  equation
    connect(a, port);
    a.e = 1;
  end Inner;
  Inner q;
  P outerPin;
equation
  connect(q.port, outerPin);
  outerPin.e = 2;
end HK;
```

Seven equations for six unknowns, and the same model with the outer
connection removed is five for four. One inner member, one outer
connection, one surplus equation - the machine's shaft in miniature,
and the place to work next. Parked here rather than repaired, because
the repair that suggests itself was measured and cost nothing.

## The stator's surplus was a reduction that reduced nothing

The per-phase half of the machines' imbalance is named, and it was not
a connection at all. The bare `SM_PermanentMagnet` with not one
`connect` in the model is already `m - 2` equations over, swept at
three, five and seven phases from one binary:

```text
bare machine, no connections   m=3: 489 for 488   m=5: 651 for 648   m=7: 813 for 808
```

So the slope lives inside the machine and nothing outside it is
implicated - the two `Star`s, the shared ground and the load were all
put back one at a time and moved the slope not at all. What the shrink
found instead is one line of `FundamentalWave.BaseClasses.Machine`:

```modelica
final powerStator=Modelica.Electrical.Polyphase.Functions.activePower(vs, is),
```

`activePower` inlines to `sum(v .* i)`, and the compiler asked about it
gave seven equations where the model wrote one:

```text
equation: smpm.powerBalance.powerStator = smpm.vs[1] * smpm.is[1]
equation: smpm.powerBalance.powerStator = smpm.vs[2] * smpm.is[2]
...
equation: smpm.powerBalance.powerStator = smpm.vs[7] * smpm.is[7]
```

Six of those are false, all seven were counted, and the surplus is
`m - 1` exactly. Worse than the refusal: where such a model did
balance, the stator power of one phase came out presented as the power
of all of them. This is the fourth appearance of the rule that a wrong
number is the worst thing this compiler can do, and the first where
the wrong number was an equation count.

### Why it happened, and why no small model shows it

`vs` and `is` are read off the machine's plugs, so both are met before
the plug array is built and both measure as scalars. The guard that
holds a reduction back until the shapes are in hand asked only whether
the argument was _itself_ such a name. Handed `vs .* is` it said no,
the product came to a single term, the fold folded one term, and the
`sum` vanished leaving the bare product. The pass that writes
whole-array equations out element by element then found two whole
arrays standing in the equation and wrote it once per phase.

The probe that settled it prints what the guard is handed:

```text
sum arg Elementwise(Mul, Ref("smpm.vs"), Ref("smpm.is")) unmeasured=true    the machine
sum arg Elementwise(Mul, Ref("mach.vs"), Ref("mach.is")) unmeasured=false   a small model
```

That difference is the whole of it, and it is why eleven synthetic
models written against this failed to reproduce it: what makes the
names late is the depth of the machine's inheritance, and a model
small enough to write out is one whose shapes are all in hand by the
time the reduction is met. This is the blind spot the notes already
name - a path that only switches on at complexity - seen from the
inside for the first time. The test is therefore written against the
rule rather than through a flattened model, and says so.

### What it measured

Two corpus runs from one binary, the change behind
`OXIDELICA_NO_LATE_REDUCTION`:

```text
                     old rule    with the fix
flatten / run        822 / 472   822 / 472
list of models       identical line for line
unbalanced rows         57          36
init-not-square rows    15          26
```

No model was lost and none was won, which is the second kind of change
the notes describe: a wall fell and the next one stands behind it. The
census is the witness that it fell - twenty-one rows left the
`unbalanced` wall, and eleven arrived at `initialization is not
square` with the rest scattering into `structurally singular`. The
machines now die at their initialisation rather than at their equation
count, and that is where the next shift on this line starts.

## An initial equation that names a current speaks about a flux

Eleven machines stood at `initialization is not square`, and the
numbers said the problem was over-determined rather than under: seven
equations and seven fixed starts for seven unknowns, ten and twelve for
twelve. A section that pins nothing would have been short of equations;
these had too many.

The probe on `SMPM_Inverter` showed why in one line. Its section reads
`smpm.is[1] = 0` and `smpm.is[2] = 0`, and the states are
`smpm.airGap.psi_ms[1]`, `psi_mr[1]`, `psi_mr[2]` and the rest - fluxes,
not currents. The rule deciding which states the section spoke about
compared the _spelling_ of the names it mentioned against the list of
states, found no match, and concluded the section mentioned nothing at
all. Every state was then pinned at its declared start, and two
equations on top of seven pinned states is a problem with nine
statements about seven unknowns.

The same fault under a different coat as the ones already recorded here:
a test on the spelling of a name standing in for a fact the structure
does record. What the structure records is the evaluation plan - `is[1]`
is computed from the fluxes, and the reachability that answers which
states each algebraic variable is computed from was already written, for
the Jacobian's colouring. The initialisation asks the same question from
the other end.

Two things had to be got right beyond sharing that walk.

**Which state an equation claims is a matching, not a union.** `is[1] =
0` and `is[2] = 0` reach the same five flux states between them, and
taking the union unpins all five on the strength of two equations. One
equation determines one state, so the augmenting-path matcher already in
the file pairs them and the rest stand where they were declared to.

**Reachability through a simultaneous block is too coarse to claim
on.** The block's reachability is deliberately generous - every input of
the block reaches every unknown of it - because a missing entry would
cost the Jacobian a term while an extra one costs only an evaluation.
Generous is the safe direction for colouring and the wrong one here: a
state claimed on the strength of that generosity is unpinned without
anything determining it. Measured, that is exactly what happened -
`FreeBody` lost its initialisation to a singular Jacobian, the only
model the first version cost. Following explicit assignments alone
recovered it and kept seven of the eight wins.

The corpus, one binary, `OXIDELICA_NO_INIT_REACH` either way: 472 to 479
run, 441 to 448 runnable, and the flatten half untouched. Seven gained,
none lost. The winners are not the machines the probe started from -
`SMPM_Inverter`, `IMC_DOL` and `SMEE_DOL` moved one storey up, off the
init wall and onto an algebraic loop that is NaN before Newton takes a
step - but the drives and the thyristor bridges behind the same wall,
which had nothing else in their way.

### Which machines actually moved

A correction to the paragraph above, and the chronicle is corrected by
adding rather than by rewriting. The fresh census says the family that
left the initialisation wall is the `Electrical` induction machines -
`IMC_DOL`, `IMC_Inverter`, `IMC_Steinmetz`, `IMC_Transformer`,
`IMC_YD`, `IMC_YDarc` and `IMS_Start` - together with the DC machines.
The two models the probe itself was written from did not move:
`SMPM_Inverter` still stands at the same wall with the same numbers,
and so does `SMEE_DOL`. Their fluxes are worked out through a block,
and reachability was narrowed to explicit assignments to buy `FreeBody`
back, so the narrowing traded away exactly the model that prompted it.
The gain of seven is real; what is not true is that the probe's own
models were cured, and a later shift standing at that wall should not
count them as done.

## A record handed to a function loses its name on the way in

`Modelica.Electrical.QuasiStatic.Polyphase.Functions.activePower` takes
`Complex v[:]` and hands each element to `real`, a function written for
one record. Every quasi-static machine in the library states its stator
power that way, and every one of them refused with `unknown variable
imcQS.vs[1]` - a name the flat model does not declare, because there is
no number called `vs[1]`; there are `vs[1].re` and `vs[1].im`.

The cause is that a function body is worked out with an empty table of
records. That is right for the body's own names - what the caller
declared means nothing inside a function - and wrong for what the
caller handed over. Once the arguments are substituted in, the body
reads `vs[1]` where it wrote `v[k]`, and whether that spelling names a
record is a thing only the caller's table knows. With nothing to ask,
the element read as a plain number, the inner call was inlined whole
instead of being spread over the elements, and the body came back
naming the record itself. A value went missing where not even a refusal
was owed, and the model died a storey lower.

The chain, walked with an eight-line reproduction rather than with the
corpus:

1. the body's own shapes carry no records, so the caller's table is put
   in view the way the caller's strings already are;
2. `record_class_of` reads a name against the table exactly as written,
   and a flattened name carries its subscripts - `vs[1]` where the
   table files the declaration under `vs`. The subscripts now come off
   from the right, the same reading `whole_record` does on the other
   side of the call, and only where the subscript is the last thing on
   the name: `vs[1].re` is a field of one and a number whatever its
   record is;
3. a declaration's value is worked out at its own site, which had the
   table to hand but did not put it in view for the bodies it calls.

With those three, a call to `activePower` written as a declaration's
value comes out as arithmetic on the fields, and the eight-line model
runs. Measured on the corpus, one binary, `OXIDELICA_NO_CALLER_RECORDS`
either way: 819 flatten and 479 run both ways, and the two lists of
models that ran are identical line for line. So the chain has a fourth
link and it has not been taken. The same call written where a record's
field value goes - `powerBalance(final powerStator = activePower(vs,
is))`, which is how every machine in the library writes it - is worked
out on a road of its own, and putting the table in view there writes
the fields out twice: the refusal becomes `unknown variable
vs[1].re.re`. The message moved, which says the road was found; where
the doubling is has not been settled, and the guess worth testing first
is that the value passes through `records_written_out` once with the
table in view and once again further down.

What was gained is a fault named and three of its four links removed at
no cost; what was not gained is a model, and the honest reading of a
change that moves no number is that the wall is still standing.

### The fourth link: a balance stated before the voltages it balances

The guess the chronicle left - that the value passes through
`records_written_out` twice - is wrong, and what is there instead is
worth writing down so the next reader does not test it again. Traced
with a print at every place a record is written out, `imcQS.vs[1]` is
written out six times and `imcQS.vs` once, and that once is the whole
fault: inside the body of `activePower` the argument `vs` is read as
_one_ record rather than as three, comes back as `vs.re` and `vs.im`,
and those two are written out a second time further down - which is
where `vs[1].re.re` is born.

The reason the body reads it as one record is that the caller's table
says `imcQS.vs` names a `Complex` and nothing in view says there are
three of them. The table of records travels into a body; the table of
lengths does not. Putting the lengths beside the records was built and
measured, with the table filtered to the names the record table knows
so that a body's own name still wins, and it does not cure the model.
The reason is the order the class is written in:

```modelica
replaceable output ... powerBalance(
  final powerStator = ... activePower(vs, is), ...);
output SI.ComplexVoltage vs[m] = plug_sp.pin.v - plug_sn.pin.v;
```

`powerBalance` is declared eleven lines before `vs`. The value of its
field is worked out where the declaration stands, and at that moment
`vs` has not been measured: it is in neither the class's own table nor
the model-wide `acc.sizes`, because the walk has not reached it. The
equations road, which runs after every declaration is measured, does
have the length - `equations road: sizes [("imcQS.vs", [3])]` - and
that is the road on which the three-element reading already works.

So the fourth link is not a missing table but a missing _order_: a
record's field value that calls a function over an array declared
later has to be put off until the class is measured, the way the
record-valued variables themselves are put off into `record_values`
and settled by `flatten_equations`. That is a change to when a value
is worked out rather than to what is in view while it is, which is why
the three links before it cost nothing and this one is a stage of its
own. The twelve models of the `vs[1]` family stand on it.

What was measured and rolled back: the lengths travelling with the
records, on all three roads, and the caller's table put in view on the
record-field-value road. Neither moves the model, and both were taken
out rather than left standing behind a switch, because a change that
cures nothing is not a link of this chain - the link is the deferral,
and it has not been built.

### The fourth link taken: a reduction that answers before it can

The deferral the chronicle called for was not needed, and what the
chain actually stood on is smaller and worse. The value of a record's
field - `powerBalance(final powerStator = activePower(vs, is))` - is
worked out where the component stands, with two tables of lengths to
hand: this class's own, which has not measured `vs` yet, and the
lengths of the class that wrote the modifier, which has. The second
was put in view only after the first reading refused.

It never refused. `sum` of a name whose length is not in view comes
back as the name itself - a deliberate lateness, written so that
`sum(rs.resistor.LossPower)` can wait for the array to be built - and
the test that decides whether to wait asks whether the name is dotted.
A bare `vs` fails that test, so the reduction summed itself to
nothing, reported success, and the second reading was never made. The
modifier arrived as the bare array and was spread over the elements:
three equations for `powerStator` where one was owed, each naming one
phase.

Measured on the corpus, one binary, `OXIDELICA_NO_WRITERS_LENGTHS`
either way: 819 flatten and 479 run with the change, which is the
floor to the digit. The machines did not start running - their
refusal changed from `imcQS.vs[1].re.re` to `imcQS.vs[1]`, which is a
different wall rather than the same one - and the honest reading of a
change that moves no count is that the family is still standing. What
it is worth keeping for is not a model but a silent wrong answer: the
power of one phase presented as the power of all three, written
without a word said. The test goes red without the change.

That is the fourth breed of the same fault these notes already record
three times over: a test on the spelling of a name standing in for a
fact the structure does not record. The name is not what says whether
an array has been built yet; the table of lengths is.

The fix is to stop treating the writer's lengths as a second attempt.
They go in with this class's own from the start, this class's entries
winning where both know a name, because the value is read here and
what the writer called `v` is not what this class calls `v`. With
that, the small model runs and `imcQS.vs[1]` is gone from the
machines - the equation for `powerStator` comes out whole, one per
machine rather than one per phase.

### The machine ten at the NaN wall are two families, not one

Probed at the point the residual is first evaluated, the eleven
`Machines` models refusing before any Newton step split cleanly:

- the DC three - `DCEE_Start`, `DCSE_Start`, `DCSE_SinglePhase` -
  stand in a block of two or three unknowns whose bad residual is a
  division by `dcee.lesigma.L`. That inductance is `Le * sigmae`, and
  `sigmae` is `0` by default in `DcElectricalExcitedData`. A stray
  inductance of exactly zero is not a mistake in the library: it says
  the stray branch is a short, `v = 0`, and the compiler instead
  solved the inductor for `der(i) = v/L` and divided by nothing. The
  cause is a structural one - what an inductor's equation means when
  its inductance is a parameter worth zero - rather than anything the
  solver can start from elsewhere.
- the `aimc` seven - `IMC_DOL`, `IMC_Inverter`, `IMC_Steinmetz`,
  `IMC_Transformer` (twice over, under two packages), `IMC_YD`,
  `IMC_YDarc`, `IMS_Start` - stand in an eighteen-unknown block round
  the air gap, where residual 11 is the one that is infinite and its
  expression is a page of arithmetic over the space-phasor transform.
  No zero parameter is in sight: the stray inductances all have
  sensible starts.

So the machine ten is not one cause, and a fix aimed at the DC three
would leave seven standing. Recorded here because the census counts
them as one row and the probe is what tells them apart.

### A coefficient of exactly zero is a relation, not a division

The DC three were taken. An inductance of exactly zero is the library
saying a branch is shorted: `L*der(i) = v` with `L = 0` means `v = 0`,
and solving it for `der(i)` divides by nothing and hands the solver an
infinity before it has taken a step. The rule is narrow on purpose: a
product whose other factor is a derivative, where the coefficient is a
_parameter_ worth exactly zero, goes to zero. A variable that happens
to be zero at this instant does not count, because it does not stay
zero over the step.

Measured on one binary, the corpus twice, `OXIDELICA_NO_ZERO_DER=1`
against the default: flatten 819 both ways, run 479 to 481. The run
list names what moved. Won: `DCEE_Start`, `DCSE_Start`,
`DCSE_SinglePhase` - the three the probe predicted, so the diagnosis
and the outcome agree. Lost: `BevelGear1D`, which now refuses as
structurally singular, no equation determining
`der(inertia2.rotorWith3DEffects.w_a[2])`. That is the expected shape
of the cost: a quenched term was the only thing defining a state, and
the definition going away takes the state out of index reduction's
reach. Three for one, and the three were a wrong number where the one
is an honest refusal.

### The aimc seven divide by a zero too, and the probe had to be fixed

The note above said no zero parameter was in sight for the air-gap
seven. That reading was wrong, and it was wrong because the refusal
named `residual 11` rather than the equation. Named, the equation
divides by `aimc.airGap.L[1,2]` - the off-diagonal of the mutual
inductance matrix, bound to a literal `0`. Same layer as the DC three,
one storey up: there the zero coefficient multiplied a derivative and
could be quenched, here it is a divisor that index reduction has
already solved through, so the quench does not reach it and the seven
still stand.

The lasting half of this is the instrument. A refusal naming a
residual's number sends the reader counting through eighteen unknowns;
naming the equation says outright what was divided by. Two shifts read
that block as "a page of arithmetic, no zero in sight", and the fixed
refusal answered it in one run.

### Quenching every zero-parameter term costs two models and wins none

The narrow rule reaches only a product whose other factor is a
derivative. The obvious widening is to drop that condition: a term
multiplied by a parameter worth exactly zero is no term at all,
derivative or not. It was built behind `OXIDELICA_NO_ZERO_TERM`, one
line of the quench, and measured on one binary over the corpus twice
without the carved-out giants.

The small model it was built on states the air-gap shape in six lines -
two fluxes, a mutual inductance matrix whose off-diagonal is zero, one
current driven - and refuses as a singular Jacobian without the
widening and runs with it. The corpus says the widening is not worth
having: flatten 819 both ways, run 481 down to 479. Lost
`Modelica.Magnetic.QuasiStatic.FundamentalWave.Examples.Components.PolyphaseInductance`,
which stops converging in fifty Newton iterations, and
`Modelica.Thermal.FluidHeatFlow.Examples.WaterPump`, whose pump port
block goes singular. Won nothing: `IMC_DOL` leaves its infinity at
time zero and arrives at a singular Jacobian one storey up, which is a
move and not a win, and the other six of the seven do not move at all.

That is worth recording rather than repeating. Quenching a term
removes an equation's grip on an unknown just as it removes an
infinity, and where the matching was using that grip the block is left
underdetermined in a way Newton discovers rather than the compiler.
The narrow rule is narrow for a reason: under a derivative the term
being zero is the whole of what the coefficient says, and elsewhere it
is not. The change was reverted; what stands is the measurement.

The air-gap seven therefore stay parked, and the map of them is this.
The divisor is not written by the flattener - it is built by the
matching, which is free to hand the air-gap equation to
`aimc.airGap.i_sr[2]` and so divides by `L[1,2]`, and `solve_cost`
ranks that choice without ever asking the parameter table what the
slope is worth. That is the same blindness the DC three had one storey
down. The honest fix is therefore not another quench but a rank that
reads the parameter table: a slope known to be exactly zero is the
dearest possible choice, not the cheapest, and the matching should
take another pairing where one exists. That is a change to
`solve_cost` and its shape, it decides which states survive reduction,
and so it is measured by the list of victims and both run lists - not
by a count.

### The rank that reads the parameter table, and the layer behind it

The rank above was built and measured. `solve_cost` now has a third
answer: `ZeroSlope`, for a slope whose every name the parameter table
values and which folds to exactly zero. That is not a cheap pairing,
it is the equation declining to mention the name at all, and it ranks
dearer than anything else so the matching falls on a pairing that
exists. Behind `OXIDELICA_ZERO_RANK`'s negative, `OXIDELICA_NO_ZERO_RANK=1`.

The eleven-line model of the air gap's shape is the witness, and it is
a witness about an answer rather than about running. Two windings with
a mutual inductance matrix whose off-diagonal is zero, one current
given: `psi2 = L12*i1 + L11*i2` determines `i2` and says nothing about
`i1`, so handed `i1` it divides by nothing. With the rank out the
model dies of a singular Jacobian; with it in, `psi1 = L11*sin(t)`
exactly. The test asserts the numbers.

What the rank does not do is win the air-gap seven, and the reason is
worth writing down because it cost most of a shift. The divisor in
`IMC_DOL` is not built by the matching at all. Index reduction builds
it, in the candidate-definition loop, where `solve_linear_for` is
called with an _empty_ parameter table - so the same blindness sits
one storey above the rank, in a place the rank cannot reach.

Handing that call the real table is four characters of change and it
was built, measured and thrown away. It works, in the sense that
`IMC_DOL` leaves its infinity at time zero and arrives at a singular
Jacobian one storey up - the same move the quench made in the shift
before, and a move is not a win. What it also does is make
`RollingWheelSetPulling` take longer than twenty minutes where it
takes fifty-two seconds, measured on one binary with the two halves
behind separate switches: rank only, 52 seconds; candidate judging on,
still running after twenty. A definition refused is a definition index
reduction must find another way around, and in a model with that many
constraints the other ways multiply.

So the rank ships and the candidate half does not. The map for
whoever takes this next: the target is `compile.rs`'s candidate loop,
the fix is not "pass the table" but "pass the table without making
reduction search", and the instrument is `--only
RollingWheelSetPulling` timed against fifty-two seconds, which answers
in a minute what the corpus answers in half an hour.

Measured on one binary over the corpus twice, with the heavy set out:
flatten 819 and run 481 both ways, and both lists identical line for
line - the two controls of the shift before, `PolyphaseInductance` and
`WaterPump`, among them. So the rank costs nothing and wins nothing on
this corpus, and what it buys is the class of wrong number the small
model shows: where a zero-slope pairing was taken and another existed,
the other is taken now. A correctness fix with a flat count is still a
fix; it is recorded as one rather than as a win.

## The parameter-without-value family, split

The census entry is read as a queue of a hundred models, and the queue
is mostly an accounting error. Probed on the corpus at 819/481, the
kind `parameter X has no value` counts 25 refusals over 19 models, and
the split by what the model _is_:

- **15 of 19 are service or base classes** - `...Utilities.*`,
  `...BaseClasses.*`, `...Interfaces.*`, `...Components.*`. A
  `DcdcInverter`, a `TankController`, a `GasForce2`: these are written
  to be finished by whoever instantiates them, their parameters are
  unbound _on purpose_, and a compiler that refuses them is right. The
  work queue does not own them.
- **4 are examples**, and only those are candidates for a fix:
  `Modelica.Blocks.Examples.Filter`, the two
  `OpAmpCircuits.Der`/`Derivative`, and
  `Modelica.Electrical.Batteries.Examples.ShowImpedance`.

And one of the four is ours outright, which the probe would not have
found by counting. `ShowImpedance` declares
`Utilities.Impedance impedance(cellData=cellData)` beside
`parameter ...ExampleData cellData(Qnom=3600, Ri=0.01)`, and `why`
answers that `impedance.cellData.Qnom` is bound to nothing. The value
is written in the model text. What does not happen is a whole-record
binding carrying its own modifiers through the component it is handed
to: `cellData=cellData` names a record that has `Qnom`, and the name
arrives without it. That is a compiler oversight and the one address
in this family worth a change.

`Modelica.Blocks.Examples.Filter` is a different kind wearing the same
words - `den1[1]` is assigned in one branch of an `if` only - and
belongs with the branch-assignment family rather than here.

So the honest size of this queue is four, not a hundred, and the
census line should say `parameter has no value (service class)`
separately from the rest the way the numerical queue was split out.
Fifteen models are refusing correctly and no change should try to win
them.

## A record handed to a declaration of its base

`ShowImpedance` says `parameter ...ExampleData cellData(Qnom = 3600)`
in plain sight and was refused for `impedance.cellData.Qnom` having no
value. The layer is `record_value_per_field` in `components.rs`: a
whole-record value is taken apart by the class that _wrote_ it and
matched against the fields of the class that _receives_ it, by count
and then by position. Those are the same class most of the time and
were not here - `Impedance` declares its parameter as the base
`CellData` and the example hands it an `ExampleData`, which extends the
base and so holds more fields. The counts disagreed, the reading came
to `None`, and the whole hand-over was dropped without a word.

Position was the wrong instrument for the question. Where the value is
simply the name of a record, there is nothing to count: the field the
target calls `Qnom` takes the value at `cellData.Qnom`, whatever else
either record holds. The fix hands the fields on by name wherever the
counting came to nothing, which is the same breed of fault the notes
already record from the other side - a test on shape standing in for a
fact about names.

It is a chain and was walked before it was measured. `ShowImpedance`
does not run on this change alone: behind the modifier stands a
flexible `:` size measured from a declaration written _below_ the one
that needs it (declaration order decides, and reordering the two lines
of the example by hand takes the model past it), and behind that
`R0 = Ri - sum(rcData.R)`, a parameter over an array of records. Those
two are the next links and are not taken here.

What the change does win is elsewhere and was found by the switch
rather than expected. Measured on one binary over the corpus twice,
the heavy set out, `OXIDELICA_NO_RECORD_BY_NAME` the only difference:
flatten 819 to 821 and the runnable half 723 to 725, with the run
counts 481 and 450 unmoved and the run list identical line for line.

The flatten diff is worth reading rather than summing, because the +2
is a +3 and a -1. Three battery examples arrive - `CCCV_Cell`,
`CCCV_CellRC`, `CCCVcharging` - and `ShowImpedance` leaves: it used to
flatten and refuse at the run for the missing `Qnom`, and now that the
value arrives it gets as far as the flexible `:` of `OCV_SOC`, which
is a flattener wall. A model moving from the run half's queue to the
flattener's is the compiler seeing further, not less; the count says
otherwise and the diff says which.

The three `NewFittings` models - `CurvedBend`, `EdgedBend`,
`ThickEdgedOrifice` - travel the same way without changing halves:
refused for `fitting1.geometry.d_hyd` having no value with the rule
off, reaching `cannot evaluate parameters` with it on, one storey up
in the same family.

## The parameter-without-value queue, measured rather than guessed

The census row was read as 114 models for several shifts and is an
accounting error twice over. The 114 was two wordings added together
at 819/367 - `parameter X has no value` at 62 and `cannot evaluate
parameters` at 52 - and at 819/481 the same two come to 25 and 40, so
65 rows over 65 distinct models. Rows and models are one to one here;
the row was never counting more models than it named.

The split that matters is not the wording but whose fault the refusal
is. Twenty of the sixty-five are service or base classes -
`...Utilities.*`, `...BaseClasses.*`, `...Interfaces.*`,
`...Components.*`, `...OpAmpCircuits.*` - written to be finished by
whoever instantiates them, their parameters unbound on purpose, and a
compiler that refuses them is right. The work queue does not own them
and never did. `refusals.sh` now prints the two apart and names the
queue half model by model, so the row cannot be read as a hundred
models of work again.

Of the five in the `has no value` half that are models of their own,
four are addressed above: `ShowImpedance` and the three `NewFittings`.
The fifth is `EngineV6`, whose `cylinder1.cylinderInclination.R_rel.T[1,1]`
is a different layer - an orientation record built by a function - and
is the named next address in this family.

The census at 821/481 confirms both numbers from a third run and prints
the split: 45 rows in the queue against 20 refusing rightly. The queue
is almost entirely one shape rather than forty-five - `pump.delta_head_init`
carries nine of it on its own, `h_start` over an ideal gas's `alow`
coefficients another five, `dp_small` over the Dissipation correlations
four more - so the next address in this family is a parameter whose
value is a call the compiler will not evaluate, not a parameter nobody
bound. That is a different illness wearing the same row, and the top of
the run half meanwhile is the algebraic loops at 29 and the square
initialization at 28.

## A matrix argument taken apart one level deep

The queue's top read as one kind - a parameter whose value is a call
the compiler will not work out - and the kind proved to be three
mechanisms rather than one. Counted over the whole of the census at
821/481, `cannot evaluate parameters` divides sixteen rows that end in
`nothing works out` from twenty-eight that end in `nothing gives a
value to`, and only the first sixteen are about a call at all. Of those
sixteen, eleven name `dgesv`: ten as `dgesv, max, min` and one alone.
The remaining five are five different names apiece - the water of
IF97 twice, a colour map, a quadrature and a Dissipation correlation.

The pump cluster was the address, and what stood there was not the
function interpreter the shape suggested. Every body along the way had
already been unfolded by the time the run asked: `quadraticFlow` fits a
quadratic through three operating points, its `Modelica.Math.Matrices.solve`
had reached the flat model as a bare `dgesv` over a matrix written out
in full, and `dgesv` has had a body written in Rust here for as long as
`outside.rs` has existed. Nothing needed building.

What refused was the taking apart of the argument. A body written here
takes numbers, and a matrix reaches it as an array of rows, each row an
array again. The side of the run that compiles a call had walked to the
leaf all along; the side that settles parameters before the run matched
`Expr::Array` once and evaluated its items, so what was offered to the
solver was three rows where nine numbers were wanted. The shape did not
fit, `answer` returned nothing, and the name came back as one nothing
works out - a refusal whose wording pointed at the body rather than at
the caller, which is why the cluster read for several shifts as needing
an interpreter.

Two answers to one question is what made this possible, so the walk is
now one function both sides call. The two lesser forms are not the same
illness: `h_start` over an ideal gas is eighteen rows of `nothing gives
a value to` and belongs to the has-no-value family, and `dp_small`
carries one genuine call refusal over a Dissipation correlation.

The change costs nothing and wins nothing, which is the expected shape
for a wall that is not the last one in a model's way. Both halves are
unmoved at 821/481 and 725/450 from one binary run twice over the same
corpus, and the two lists of which models flatten and which run are
identical line for line, so there is no scattered loss hiding under an
unmoved total either. What moved is the wall: `PumpingSystem` refused
with `cannot evaluate parameters` under the switch and refuses with
`cannot differentiate this expression` without it, one storey up in the
same model. Eleven of the sixteen rows in the call half of the queue
are that kind, and the kind is now absent from the register while the
models behind it stand at whatever was next.

## A gas constant lost to a namesake

The largest cluster of the has-no-value family was nineteen fluid
models refusing over the fields of an ideal gas's `data` record -
`data.R_s` alone in six of them, the whole list `H0` through `blow` in
the rest. The values are written out as constants in
`Modelica.Media.IdealGases.Common.SingleGasesData`, so the reading for
several shifts was that a record with ready values was not carrying
them to its fields, and the mechanism was unknown.

It was not the record. Every NASA ideal gas states its gas constant as
`R_s = R_NASA_2002/Air.MM`, naming the record constant beside it, and
`Modelica.Media.Air` is a package of that same name one branch over.
The head resolved - to the package - the package had no `MM`, and the
lookup answered nothing rather than asking whether the scope held a
record constant of the name. One namesake, and the gas constant of
every ideal gas in the standard library was unreadable. The rule the
notes already state from the other end applies here too: a name that
resolves to something with nothing to say is not a resolution, and a
class that answers nothing has to send the question on rather than end
it.

Probing found it where three replicas failed to. Seven hand-written
models of the shape all passed, because none of them had a namesake to
trip over, and the twelve-line replica that did reproduce was reached
by shrinking the real model rather than by growing a synthetic one -
the fault was two layers below where the refusal pointed. One of the
replicas was passing for a worse reason still: a package with two
models in it, where `simulate` ran the other one, and the reading
"this shape works" was drawn from a model that never touched the
shape. A measurement of the wrong model looks exactly like a
measurement of the right one.

A second fault was found on the way and measured rather than kept. A
short class definition may carry redeclarations - a vessel writes
`replaceable model FlowModel = Flow(redeclare package Medium =
Medium)` and then declares `FlowModel flowModel`, so the alias is the
only place the medium is named - and the parser reads the modifier
list and keeps only half of it, dropping the redeclaration at the
door. Carrying it through is right by the specification and cost
forty-one models: 821 flatten to 780, on a corpus run against an
unmodified binary for the baseline. So it is reverted and written down
here instead. A redeclaration that was being dropped is a definition
that was not being made, and supplying one changes what index
reduction can reach through, which is the shape these notes already
name: such a change is not wrong for costing models, but forty-one is
not a small number and nobody has yet walked the victims. That is the
next shift's work, not this one's.

The namesake fix alone is a correctness fix with a flat count.
Measured on one binary run twice over the corpus, 821 flatten and 481
run either way, and identical to an unmodified binary's own run as
well - all three lists the same line for line: no model is won and
none is lost. What moved is the wall and the clock. The
junction test that stood at `cannot evaluate parameters` now stands at
an unknown variable `source2.medium.data.Tlimit` - one storey up, the
gas data read and the next wall behind it - and flattening that one
model fell from forty-six seconds to nineteen, and the whole corpus
from 3112 seconds to 2199, because the road that used to fail also
used to settle twelve hundred gas records in order to say so. A
correctness fix that takes a third off the flattening time is still
recorded as a fix rather than as a win, but the time per model is one
of the numbers the floors hold, and it moved the right way.

And the measurement itself carries a lesson worth more than the fix.
The first pass reported both halves at 780, which read as "the change
costs nothing"; the true baseline was 821, and what the switch turned
off was one of the two changes in the tree. A switch covers the change
it was written for and says nothing about the one beside it, so two
changes measured through one switch are not measured at all. The
baseline belongs to an unmodified binary, and the corpus says so in
eleven minutes.

## The gas data chain, second and third links

The namesake fix took the gas cluster one storey up and left it at
`unknown variable source2.medium.data.Tlimit`, and the probe walked
the rest of the chain in a single small model. Seven lines of live
library - a `DryAirNasa` medium and a `BaseProperties` under it -
refuse exactly as the nineteen do, in two seconds rather than in the
five minutes a corpus pass costs.

Two links stood behind that refusal, and only the first has been
taken. Every NASA ideal gas is written `extends SingleGasNasa(data =
Common.SingleGasesData.N2)`: the record constant is given the _name_
of another record constant rather than a constructor call. The reader
of a record's fields knew two shapes - a modifier list on the
declaration, and a constructor the binding comes to - and a bare name
was neither, so the whole coefficient table of every gas read as a
variable nothing declares. Following the name is the same question one
level along, and it is now asked. A second fault sat in the same
function and was found by the same probe: the binding was looked for
under the record's _whole written path_ where the gathering knows it
by its own name, so the road could only ever have answered for a
record with no path at all.

With that, `M.data.Tlimit` and `M.data.R_s` read through a package
alias for every single gas and for `DryAirNasa` - 1000 K and 287.05
J/(kg.K), the numbers the test holds.

The second link is not taken and is written down here for the next
shift. Inside a medium's own functions the record is named bare -
`data.Tlimit` in the body of `specificEnthalpy`, and `data` handed
whole to `h_T` - and a bare name that stands for a record is folded by
nothing: it is neither a number nor a list, so every road in the
substitution passes it by, and it reaches the flat model with the
instance path on its front as `medium.data`, which nothing declares.
The field lookup itself answers correctly when asked; what is missing
is the step that gives the bare name its path before prefixing. The
minting road is the shape to copy - `mint_asked_as_constant` does
exactly this for a scalar of the medium on the mark - and the record
twin of it is the work. Writing the path out raw was tried first and
is wrong: the name is prefixed all the same unless it goes through the
minted ledger that `flat_name` consults.

Measured through a switch on one binary, the corpus twice: 821 flatten
and 481 run either way, 725 and 450 of the runnable, and the flattened
list and the run list identical line for line. No model is won and none
is lost - this is the second link of a chain and not its end, so the
number that moves is the one the last link will move. What it buys is
that the field lookup is now right for a shape it was silently wrong
for, and the remaining wall is named above rather than guessed at.

## The gas data chain, the third link

The third link is taken, and it was not where the map said. The map
had the fold going into the constant substitution, beside the minting
road: give the bare name its path, let `flat_name` leave a minted name
alone, and the chain ends. Built there, it read `821 flatten` down to
`803` and the flattening time from 2109 to 3705 seconds, because a
fold at that depth fires for every bare name a library writes and not
only for a record of the medium. Eighteen models were lost to it,
`ModelicaTest.Fluid` and `ModelicaTest.Media` almost entirely, and
narrowing the gate - a mark, then a mark that is the declaring
package - moved none of them back. That is the sign that the layer
was wrong rather than the condition.

The place is where an argument is bound to a record input. A record
handed over by name is already taken apart there, field by field, so
that the body reads `data.Tlimit` off the caller's own name; the only
thing missing was that the caller's name here is a constant of the
medium rather than a variable of the model. Found where it is
declared and handed over in the record's own field order, the body
reads numbers and no path survives into the flat model. Nothing else
in the compiler changes, which is why the eighteen came back.

One shape had to be read twice. The same call is reached from the
body the medium wrote, where the argument is `data`, and from the
equation flattening built out of it, where the prefix is already on
and the argument is `medium.data`. A tail is taken only where what
stands in front of it is not a class, so a name that resolves on its
own is never shortened - the rule the compiler already holds about
names shortened to their tails.

Measured through a switch on one binary, the corpus twice: 821
flatten either way, run 481 against 482, runnable 725 either way and
450 against 451. The flattened list is identical line for line and
the run list gains `Modelica.Media.Examples.IdealGasH2O` and loses
nothing. Flattening came down from 2109 to 1743 seconds, which is the
cost of the paths that used to survive.

## The top of the run register is three families, not one

The census counts 50 refusals across the two rows that say a variable
is unknown - 33 `unknown variable X in equation` and 17 bare. Read as
one number that is the largest row of the run half, and work aimed at
it would look like the obvious next thing. Grouped by the name that
goes missing and by the package that holds the model, the 50 come
apart into three families with nothing in common but the wording of
the refusal:

| name                 | models | where                                               |
| -------------------- | ------ | --------------------------------------------------- |
| `data`               | 14     | `Media`, `ModelicaTest.Fluid`, `ModelicaTest.Media` |
| `V_flow_nominal2[1]` | 10     | `ModelicaTest.Fluid.TestComponents.Machines`        |
| `vs[1]` and its like | 11     | `Magnetic.QuasiStatic.FundamentalWave`              |

Each was probed to a representative and each stands on a different
layer, so none of the three is reached by work on either of the
others. The count of kinds was a lower bound on the number of
families once again, and by a factor of three.

### `data`: a chain three links deep, the last of them architectural

The `data` family is the one the previous shift's fold did not
finish. `Modelica.Media.IdealGases.Common.SingleGasNasa.T_h` writes

```modelica
T := Modelica.Math.Nonlinear.solveOneNonlinearEquation(
  function f_nonlinear(data = data, h = h), 200, 6000);
```

and the chain was walked to its end with a twelve-line model that
refuses in half a second, `Medium.T_h(h)` over `SingleGases.N2`:

1. `specialized()` in `arrays.rs`, which turns a handed-over function
   into a copy with numeric inputs, appends each filled-in argument to
   the outer call exactly as written. `data` is a bare name of the
   medium's own constant, so it leaves the medium's scope and reaches
   the flat model as `data`, declared nowhere. That is the refusal the
   14 models carry.
2. Folded to the record it names, the argument then goes through an
   input of the copy - and an input is one number where a record is as
   many as it declares fields. The refusal becomes `an array of shape
[7] is used where a scalar is expected`.
3. Written into the body instead of through an input, the value
   reaches the walk, and there the third link stands: `DataRecord`
   holds a `String name` and four arrays, and `records_as_arrays` in
   `carried.rs` takes only a record whose fields are every one a plain
   number. The refusal becomes `"N2" is a String, and a String has no
value a step can carry`.

The third link is not a local fix. A record of mixed fields is a shape
the walk has no way to hold, and giving it one is a change to what a
carried body is - which is why the chain is parked here as a map
rather than taken. Qualifying the name instead of folding it was tried
and measured on the same model: `Modelica.Media.IdealGases.SingleGases.N2.data`
is then unknown in its turn, because the flat model declares no such
component either, so that road leads back to the same wall by a longer
way.

Worth noting against the temptation to call this a walk problem
generally: the same record travels perfectly well when it is written
out in full. `Functions.h_T(SingleGases.N2.data, 300 + time)` runs and
answers. It is the partial application that has nowhere to put it.

### `vs[1]`: a record computed rather than named

The `vs[1]` of this register is not the `vs[1]` of the machines a week
ago. That one was a modifier arriving as a bare array and spread over
the elements, so the power balance got one equation per phase where it
was owed one; the fix was to read the writer's lengths with this
class's own, and it left the machines refusing at a wall with the same
name on it. This one is a layer below, in how a call written for one
record is spread over an array of them, and the earlier rule does not
reach it. Same name in the refusal, different breed - which is the
scar about a kind being a row and not a family, seen once more through
the name rather than through the wording.

`Modelica.Electrical.QuasiStatic.Polyphase.Functions.activePower` sums
`real(v[k] * conj(i[k]))` over the phases, and `conj` is written for
one `Complex` and handed an array of them. The language vectorizes
that, one call per element, and `spread_of_records` in `arrays.rs` is
what counts the elements. It recognised an element by its name: `vs[1]`
of a declared array is a name the table of records knows. A record
that was _computed_ has no name left - `{conj(vs[k]) for k in 1:3}` is
three records each already written out as its two fields - and read by
the name alone the three were taken for the fields of one and refused
for being three where two were wanted. Where the value did reach the
modifier, it was spread over the elements instead, and every equation
of the power balance named `vs[1]`, which the flat model does not
declare.

What says a value is one record there is its depth and its width: an
array of exactly as many plain values as the input's record declares
fields. Measured on the corpus, one binary, `OXIDELICA_NO_WRITTEN_RECORDS`
either way, the heavy models carved out as usual: flatten 821 to 827,
run 482 to 484, runnable 725 to 731 and 451 to 453. Both lists diffed
before against after, and there are no victims at all - the change
only adds. What it adds to the run is `TestSensors` of the polyphase
sensors and `IMC_Inverter`; the other six reach flattening and stop at
initialization that is not square, which is a wall of its own and the
next thing to ask about for this family.

The test holds a number rather than a flattening: three phasors
conjugated element by element give one, two and three back, where a
call spread wrongly gives the first phasor three times.

## A check that travelled out of a body and left its names behind

`V_flow_nominal2[1]` was the second family of the run register, ten
models of `ModelicaTest.Fluid.TestComponents.Machines`, and the name
belongs to nobody the flat model declares: it is a local of
`Modelica.Fluid.Machines.BaseClasses.PumpCharacteristics.quadraticFlow`,
which squares three nominal flow rates and fits a quadratic through
them with `Modelica.Math.Matrices.solve`.

The layer is the checks an inlined body sets aside. An `assert` in a
function body holds every time the function is called, and a call
inlined into an equation answers with an expression, which has nowhere
to put a check - so `inlining.rs` leaves the check in a pile the class
being instantiated takes up afterwards. `Matrices.solve` asserts that
the system it was handed was not singular, and that check is written
over the matrix argument, which here is `[ones(3), v, v2]` with `v2`
the caller's local. The outputs of a body are substituted into the
caller's names on the way out; the checks were not. So the check
reached the run naming a local of a body nobody kept, and the run
refused the model for a variable it had never heard of.

Two piles carry checks and both needed the same treatment: the list
handed to `worked_body`, and the thread-local `SET_ASIDE` that a call
nested inside this body writes to directly. The mark is taken before
the locals are worked out rather than before the body runs, because a
local's own value may make a check - which is exactly this case: the
`solve` is in a declaration, not in a statement.

The small model is twelve lines and shows it in one second; a
synthetic `solve` with the matrix written out element by element does
_not_ show it, because the local is then substituted before the call
is reached. The reproduction needs the local to reach the body whole.

Measured on the corpus, one binary, `OXIDELICA_NO_CHECK_NAMES` either
way, the heavy models carved out: flatten 827 both ways, run 484 both
ways, and both lists diffed before against after come out identical.
The ten models are one wall further along - `TestWaterPumpDefault` now
refuses with an algebraic loop that diverges, and one of the ten with
a `der(pump.medium.h)` that is not a state. So this is a kind removed
rather than a model won, and the row it emptied is replaced by walls
that already had rows of their own.

## A `fixed = true` on a variable index reduction demoted

`initialization is not square` was the top row of the run register at
34 models, and the probe says it is one layer rather than one
wording. Every one of the 34 misses the same way: the conditions and
the pinned starts come to _more_ than the unknowns, from `+1` on
`Blocks.Examples.PID_Controller` to `+12` on `IMS_Start`. A row that
misses in one direction only is a row with one cause behind it.

The cause is a condition being dropped and then made up for. A model
writes `inertia1.a(fixed = true, start = 0)` on an acceleration, and
index reduction demotes `a` to an algebraic of the reduced model.
After that the declaration is carried by nothing: the `fixed` flags
handed to the initialisation are read off the _states_, and the
`initial equation` section never mentioned it. The count of
conditions came out one short, so the filling in below pinned a state
the section does determine, and the problem was then over-determined
by exactly the conditions that had gone missing.

It belongs in the system as an equation, not as a pinned state: it is
a condition on a variable the plan computes, and it is satisfied by
moving the states until the computed value agrees with the declared
one. The residual it contributes is exactly that difference, and the
compiler already had the list - `fixed_starts`, which
`check_block_regularity` reads to complain when the constraints
disagree with a declared value. That complaint was the same fault
seen from the other side: where the filling in happened to balance,
the model was not refused as lopsided but solved with the declared
condition ignored and then found to contradict it.

Measured on the corpus, one binary, `OXIDELICA_NO_DEMOTED_FIXED`
either way, the heavy models carved out: flatten 827 both ways, run
484 against 492. Eight models won and none lost - `PID_Controller`,
four of `Magnetic.FundamentalWave`, and three of
`Magnetic.QuasiStatic.FundamentalWave`, which is the six that were
parked as a family. The other 26 of the row stand one wall further
along: `SMEE_DOL` now counts 7 conditions against 4 pinned starts for
a system it still does not balance, which is a second question about
how many conditions a machine's section really writes.

## The NaN at the start of a loop is a row, not a family

The run register's top row names twenty-seven models whose algebraic
loop cannot be evaluated where it starts. Probed at three
representatives, the row is at least three layers, and the reading
that "the initial guess is not seeded from start attributes" is true
of one of them and false of the others.

`SaturatedInductor` starts its block from four clean zeros and
`R_m = 1/G_m` is minus infinity there. The retry from off the zero
already in the solver does fire - measured, all four magnitudes - and
what stands behind is `singular Jacobian` at three of them and
`underdetermined algebraic loop` at the fourth. So the wall that
counts for this model is not the NaN at all; the NaN is a doorway with
another wall a step past it.

`TestWaterPumpDCMotorHeatTransfer` is the opposite. Its block starts
from the real start attributes - `pump.medium.p = 100000`,
`pump.medium.h = 84011.8`, `pump.rho = 1` - and the NaN arrives from
_outside_ the block: twenty-six inner values are already not numbers
when the residual is first asked for, headed by `Valve.m_flow`. The
seeding hypothesis says nothing about this model.

`HeatingNPN_NORGate` is a third: the unknowns of its loop are
`der(T1.vbc)` and `der(T2.vbe)`, derivatives rather than variables,
and no magnitude of retry moves it.

Which gives the general shape again, from a new side: the counter
splits a family by its wording, and here it has _joined_ three
families under one wording. Probing put them in different layers in
minutes; the row would have been worked as one family for a shift.

### And one real defect found on the way

Chasing the pump's NaN to its source led out of the solver entirely.
`Valve.m_flow` is computed through `Modelica.Fluid.Utilities.regRoot2`,
whose body is written `y := smooth(2, if x >= x_small then ...)`, and
the run answered `unknown function 'smooth'`.

`smooth` and `noEvent` are hints about continuity and event
generation, and the value is the argument. Flattening strips them from
the model's own equations. A function body is not flattened: it is
carried whole and walked at the run, where the hint is still written
where its author put it - and the run's evaluator had no rule for it.
The refusal named the wrong thing entirely, telling a reader that the
standard library used a function this compiler had never heard of.

The test for it has to defeat inlining: a body simple enough to be
substituted whole never reaches the walk, and the walk is where the
fault was. A `while` loop in the body is enough.

### The refusal that named nothing, one layer further down

The `smooth` repair moved three models out of the NaN row -
`TestWaterPumpDCMotorHeatTransfer`, `TestWaterPumpRecirculation` and
`TestValvesCompressibleReverse`, named by diffing the row model by
model rather than subtracting its count. Two of the three landed on
something new: `assertion failed at t = 0.000000: ?`.

A refusal spelled `?` names nothing, which is the one thing a refusal
may not do. The census had twelve models standing on that exact line,
and the wall behind it turned out to be one family with one cause.

The mechanism is where the text was read. An `assert` message was read
for its text at parse time, and Modelica builds a message by joining
pieces with `+`. A piece that is not a literal has no text there, and
stood as `?`. The standard library's boundary check is written
`assert(X_boundary[i] >= 0.0, message)` inside a function whose
`message` is an _input_: nothing about it can be known where the body
is written, because the text arrives with the call.

So the message is held as it was written and read for its text where
the body is walked with its arguments substituted. What the twelve
models were trying to say all along then comes out:

```text
The boundary mass fractions in medium "?" in model "Boundary_pT"
do not sum up to 1. Instead, sum(X_boundary) = ?:
```

Which is a real defect in the compiler, now stated in its own words
instead of as a question mark - and the remaining `?` marks in it are
the same fault one layer up, in `Streams.error`, where a message is
built from a `String(x)` the walk never evaluates. That layer is
parked rather than guessed at.

The change costs nothing and buys no model: both corpus halves from
one binary give 827 / 492 / 731 / 461, and the two run lists are
identical line for line. It is a wall named, not a wall removed - the
twelve still do not run, and now they say why.

### The wall behind the name: a check that escaped its branch

Naming the refusal was what made the next question askable, and the
answer was not in the message at all. `Modelica.Utilities.Streams.error`
is written in the standard library as `assert(false, string)` - the
whole of its body. A model reaches it through a branch:

```modelica
if nX > 0 and abs(sum(X_boundary) - 1.0) > 1e-10 then
   Streams.error("... do not sum up to 1 ...");
end if;
```

Where an `if` in a body cannot be decided by the compiler, flattening
works out both branches and merges what they assign. What they
_check_ was not merged: a check made inside a branch was carried out
of the `if` bare, and a check that reads `false` outright fires at the
first step of every run whatever the condition says. The composition
was right all along - `sum(X_boundary)` is 1 in every one of those
models - and the compiler refused them for a branch none of them
takes.

The rule was already written for the expression side, in `arrays`,
where a call inside a branch has its checks guarded by the branch's
condition. The statement side did not have it. The repair is the same
one: a check a branch makes becomes `not condition or check`, so it
holds wherever the branch is not taken. An `else` branch is left
alone - what it holds under is "no condition before it", which is not
one expression here, and a guard written as a guess is worse than
none.

Measured from one binary, both corpus halves with the giants carved
out: 827 / 492 / 731 / 461 before, 827 / 504 / 731 / 473 after. Twelve
models gained and none lost - the eleven of the boundary check, and
`ModelicaTest.Utilities.TestWriteFile`, which stood on the same
mechanism through a different shout about a file it could not remove.
That last one is the census's own lesson paid forward: the row of
twelve read as one family was two by its texts, and both turned out to
stand on one cause a layer below the text.

## Five hundred, and the shape of the two accelerations

473 of the 911 runnable examples in the measured corpus run - more
than half - and 504 of all 1040 (three giants are measured on their
own schedule and are not in these counts). The pace is worth
recording as a pace rather than a total: a hundred models a day three
weeks ago, one or two a day for a month after that, and six to twelve
a day again now. The second
acceleration is not the work getting easier. It is three instruments
arriving - a census precise enough to name families, a measurement
cheap enough to make twice, and the habit of sorting a row by the
layer it refuses from rather than by the words it refuses with.

## `cannot evaluate parameters`: one row, six forms, two layers

Twenty models stood at this row. The row is a work queue by the
instrument's own reckoning, and the question worth asking of it was
not what the twenty have in common but what separates them from the
many models whose parameters are equally unworkable and which run
anyway, carrying the value into the run instead.

The compiler answers that itself, in the `because` clause it prints.
Counted over the raw census, the twenty divide in two:

```text
 7  nothing works out `<call>`
12  nothing gives a value to `<names>`
```

Those are two layers and not two wordings. A call nothing works out -
`waterBaseProp_ph`, `dp_curvedOverall_DP`, `quadratureLobatto`,
`ColorMaps.jet` - is a body the compiler declines to evaluate before
the run, and the model dies because a parameter's value needs a number
now. A name nothing gives a value to is something else entirely: the
value is workable, and the name in it is one the flat model never
built.

The second layer splits again by probe, not by text:

- a field of an array of records, read as a slice: `cellData.rcData.R`
  in Batteries (2 models);
- a doubled prefix: `pump.pump.V_flow_op` where the declaration is
  `pump.V_flow_op` (2 models, and the same breed as the `stop1.stop1.s`
  pair standing in the `unknown variable` row);
- a record of a medium handed down whole - `data.H0` and forty more
  fields - where the flat model has no such component (2);
- MultiBody's analytic joints, `jointUSR.e2_ia` and its rod (3);
- a pipe's `dxs`, which the `why` probe shows is not refused at all in
  its sibling models but carried into the run (1).

That last one is the answer to the question the row was asked. What
separates a model that carries an unworkable parameter into the run
from one that refuses is not the shape of the binding. It is whether
the name in the binding exists in the flat model at all. A value the
compiler cannot work out is carried; a value naming something that
was never built cannot be, because there is nothing for the run to
read. So the row is not one barrier with twenty models behind it but
two layers, and the second is five different ways of failing to build
a name.

### A field of an array of records, read as a slice

The smallest of the five, and the one with a twelve-line
reproduction:

```modelica
model M
  record I parameter Real R; end I;
  parameter I items[2] = {I(R = 1.0), I(R = 2.0)};
  parameter Real total = sum(items.R);
```

`total` refused with "nothing gives a value to `items.R`". Three
neighbouring spellings run: `items[1].R + items[2].R`, and
`parameter Real rs[2] = items.R` followed by `sum(rs)`. So the fault
is neither the record array nor the slice nor the sum, but the three
together in a scalar parameter's binding.

The cause is a test of a name's spelling standing in for a fact about
the world - the breed these notes have now met four times. A reduction
over a name whose array has not been built yet must be left standing
until the shapes arrive, and the test for "has not been built yet" was
`named.contains('.') && !sizes.contains_key(named)`. But a member of
an array is never in `sizes` under its own full name: `plug.pin.v` is
measured by finding the longest measured prefix, which is exactly what
`member_of_array` does one line later in the walk. So every member of
every measured array read that way was judged unmeasured, the sum
waited for a shape already in hand, and what reached the run was the
bare name.

The repair asks the same question the walk asks: is some prefix of
this name a measured array. Where the reduction is genuinely early -
`rs.resistor.LossPower` before `rs` exists - no prefix is measured
and the sum is still left standing, which is the case the test was
written for.

Measured from one binary, both halves, giants carved out:
827 / 504 / 731 / 473 both with the repair and without it, and the two
run lists identical line for line. Zero models, and the zero was
expected before it was measured: this is a link removed from the
middle of a mapped chain, which moves no number by construction. The
Batteries models die one step further along, at
`battery.cellData.rcData.R` in an array component's modifier, which a
small model does not reproduce - `Res rs[n](R = cellData.rcData.R)`
through a nested record runs correctly today. So the chain has at
least one more link and its map is not finished; the link taken is
kept rather than reverted, as a chain half-walked is the one thing
these notes say not to undo.

### The doubled prefix, found by shrinking the real model

`stop1.stop1.s` in Translational's `Friction`, and
`massWithStopAndFriction.massWithStopAndFriction.s` in `HeatLosses`:
one fault, two models, and the layer is `code.rs` refusing a name the
flat model never declared. The `why` probe confirms it: `stop1.s`
exists and carries every equation it should, while `stop1.stop1.s`
is declared nowhere.

Six small models were written against the shapes the source suggested
and all six ran correctly: a class extending one declared inside
itself, the same with `encapsulated partial`, the same reaching a
grandparent through an `import`, `reinit` in a `when`, `reinit` in a
`when` under such an `extends`, and a `fixed` modifier written at the
use site onto an inherited state. So the doubling is not in any of
those on its own, and the reconnaissance stops here with the layer
named rather than with a guess about the cause. The next probe should
shrink the real model rather than grow another synthetic one, which
is what these notes already say and what six passing models have now
said again.

And that is what found it, in one pass. The real model cut down is
twelve lines: a component holding a state, and a check inside a `when`
naming that state. The condition was resolved - which puts the
component's prefix on its names - and then expanded again, which put
the prefix on a second time. Neither `initial()` nor the message built
with `String(s)` had anything to do with it, though both were in the
six synthetic models and in every earlier guess; any `when` check
naming a variable of its own component died the same way.

Worth recording as a method rather than as a fix: six synthetic models
written from the outside all ran, and one real model cut in half four
times gave the cause in a minute. The synthetic model tests the layer
you already imagined.

### The Batteries chain, walked to its end

The link left standing last shift had two more behind it, and both
were found by shrinking the real model rather than by writing a
seventh synthetic one - which is what the previous note said the next
probe should do.

The first: a record handed down whole, `cell(cellData = cellData)`,
had its fields settled from the receiving record's own defaults
alone. `nRC` came out as the declaration's 1 rather than the 2 the
site wrote, the array below it was built one element long, and what
reached the run was `cell.cellData.rcData.R` - a name with no
subscript, which is how a slice of an array that was never built
looks from the outside. The refusal named an unevaluable value, and
the truth was an unbuilt name: a value that cannot be worked out is
carried into the run, and a name that was never built has nowhere to
be carried to.

The second: the record the site redeclared through an `extends` -
`extends BaseCell(redeclare CellData cellData)` - was read through
the interface's own declaration, which holds none of the derived
record's fields at all. Both halves of the answer are the same rule
from two ends: what a field of a record is worth is decided by what
the site wrote, not by what the declaration it was written on says.

The third link is in the run rather than in flattening. `initial()`
is rewritten to the slot the event machinery fills, and an expression
built after that rewrite - the sensitivity of a constraint, produced
by differentiating here - still carries the call as the model wrote
it. The slot is already in the table by then, so the compiler reads
it rather than refusing a name it does supply.

Behind all three stands a fourth wall that is not of this family:
`CCCV_CellRC` now reaches an initialisation that is not square, five
initial equations against three unknowns. That is a different layer
and is parked, named, rather than guessed at.

## `inner`/`outer`: the top of the model, not the order of the walk

Thirteen models of the flatten register stood under one sentence -
`outer X has no inner declaration above it` - and the register's own
rule says a heading is not a family. Probed, this one was: every
thirteen have the same form, and the form is not the one recorded
against them.

The entry for F8 called it an order of instantiation: the `inner` is
there, and the walk looks for it before the tree above has been built.
It is not that. The thirteen are `Utilities` and `BaseClasses`
helpers - `Noise.Utilities.ImpureRandom`, `Loops.Utilities.Cylinder`,
`HeatExchanger.BaseClasses.BasicHX`, `StateGraph.Examples.Utilities.
TankController` - written to sit inside a model that holds the shared
instance and saying `outer GlobalSeed globalSeed` or `outer System
system` on that understanding. A library check makes each of them the
top of a model of its own. There is nothing above them at all, so the
`inner` does not exist to be found late; it does not exist.

Which is a case the language has an answer for. MLS 5.4 says that an
`outer` with no `inner` above it has one declared at the top of the
model with its class's own defaults, and a diagnostic given. That is
not a guess dressed as a rule: it is what the specification prescribes
and what every other tool does, and it is why the standard library's
helper classes are written the way they are.

Minted, the thirteen become nine that flatten and one that runs -
`TankController` - measured off one binary with the behaviour behind
`OXIDELICA_NO_MINTED_INNERS`: 828/505 against 837/506, with the two
lists diffed and no victim anywhere. The four that did not move are
still refused, one wall further along, which is the register's usual
shape: the entry empties and what stood behind it is a different
layer.

The general form is worth keeping apart from the fix. A refusal that
names a missing declaration has two readings - it exists and was
looked for wrongly, or it does not exist and something must supply
it - and those are different defects at different prices. The probe
that tells them apart is five lines: a component with an `outer` and
nothing above it. Three weeks of a wrong entry in this document would
have been one second of that model.

### The stack of records, scouted rather than taken

`CCCV_Stack` and `CCCV_StackRC` refuse by saying that `cellData` has
six elements but its value has 774, and the two numbers say what the
fault is without a probe. The declaration is `CellData cellData[Ns,
Np]` with `Ns = 3` and `Np = 2`, so the target is six records. The
binding is a comprehension whose every entry is a whole record -
`{{... then cellDataDegraded else cellDataOriginal for kp in 1:Np}
for ks in 1:Ns}` - and 774 is 6 times 129, the scalar field count of
one `CellData` with its nested `rcData`. So the target is counted in
records and the value in scalars, and the comparison is between two
different units.

That is the same layer as the whole-binding work of the Batteries
chain, one storey up: there a record was handed down whole to a
scalar declaration, here six of them are handed to an array of six.

That reading was right about the arithmetic and wrong about the cause,
which is worth recording because the arithmetic was so persuasive. The
units are not records against scalars: 774 is 6 times 129, and one
`CellData` holds 128 numbers the count allows plus the one inherited
`constant String CellType` it does not. A record named whole and
written out brings every field its class declares; the count of how
many numbers one record holds leaves the constants out, on the grounds
that a constant belongs to the class rather than to any value of it.
Both statements are true, and only one of them can be the length.

Seven synthetic models were written before that showed - records in
records, comprehensions, redeclarations, unsized table fields - and
every one of them passed, because each tested the layer that had been
imagined. What found it was the real model with its components thrown
out one at a time until twelve lines still refused, and then a single
substitution: replacing the library's `CellData` with a local record
of the same shape made the refusal vanish, and putting back the one
thing the local copy lacked - a base holding a constant - brought it
straight back.

### Where the fix does not go

The obvious place is the writing out: drop the constants there and the
two counts agree. Measured, that costs nine Spice3 models, because a
function input written for a record wants every field the record has
and `mosCalcNoBypassCode` then gets none. The counting site is the
local one: allow both lengths, since which arrived says how the value
was built, and answer separately the question of whether a value may
set a field. 837/506 before and after, no victim in either list.

Behind it stood a second wall in the same two models, and it is a
rule true of one kind of thing applied to another. A declared bus
member nothing connects to is worth zero - 9.1.3, a potential variable
with no connections - and the loop that supplies those zeroes did not
ask what the member was. A stack's bus states `parameter Integer Ns`
and hangs a cell bus off it; zeroed, it lost the count the model wrote
and the model was refused for a count below the minimum its own
declaration states. A parameter is not a potential variable and has a
value already.

The chain ends at a third wall of another family: both models now
reach `ModelicaStandardTables_CombiTable1D_getValue`, a table function
written outside Modelica, which is the same barrier `ShowImpedance`
and the OCV tables stand at. That is why the counts do not move.

### The rcData sub-family, scouted

`BatteryDischargeCharge` and `CCCVcharging` flatten and refuse at the
run with `cannot evaluate parameters`, and what nothing gives a value
to is `battery2.cellData.rcData.C`, `.R`, `.T_ref` and `.alpha`. The
probe says those names are declared nowhere in the flat model, which
is exactly right: `rcData` is `RCData rcData[nRC]`, an array of
records, and `CellRCStack` reads it without a subscript -
`final R = Ns*cellData.rcData.R/Np` over `resistor[cellData.nRC]`,
one element apiece. So a name for the whole array's field survives
into the run, where every name must be one the flat model declares.
This is the array-of-records reading of the same whole-binding layer,
and it is scouted rather than taken: there is no shrunk model yet.

### The rcData chain, walked to its end and parked

The scouting note above is superseded: the chain was walked, four of
its five links were built and measured, and the whole of it was
reverted. What follows is the map, so that the next attempt starts
from the fifth link rather than from the first.

The shrunk model is thirty lines and shows the whole fault. A record
written as a base and a list of modifiers, a second record extending
it, an interface record redeclared on a stack, and a field of the
array read as a slice:

```modelica
record Elem Real R = 1; end Elem;
record BaseData parameter Real Ri = 1; end BaseData;
record CellData extends BaseData; parameter Integer n = 1;
  parameter Elem a[n] = {Elem(R = 0)}; end CellData;
record ExampleData extends CellData(n = 2,
  a = {Elem(R = 3), Elem(R = 4)}); end ExampleData;
partial model BaseStack replaceable parameter BaseData cellData; end BaseStack;
model Stack extends BaseStack(redeclare CellData cellData);
  Leaf leaf[cellData.n](final R = 2 * cellData.a.R); end Stack;
model M parameter ExampleData cellData; Stack battery(cellData = cellData); end M;
```

Two things had to be in the model before it would fail, and both were
learned by bisecting rather than by reading. One storey is not
enough - the same record read directly flattens correctly - because
the fault needs the record to be instantiated a _second_ time under
the site's path, which is what the redeclare causes. And the slice is
not the fault but its symptom: what goes wrong is the length.

The links, in the order the probes found them:

1. `fields_of_a_record` in `instantiate.rs` reads an inherited field's
   binding from the base's declaration and never looks at what the
   record's own `extends` wrote about it. `n` settles at 1.
2. `settle_parameters_early` writes the base's default over a value
   already settled under the same path, on the second visit.
3. `settle_parameters` does the same thing at the end of its round.
4. `instantiate_one` in `components.rs` does it a third time.
5. The value itself. With the length right, `a` is measured two long
   and its value is still the base's one-element `{Elem(R = 0)}`,
   which comes apart into four numbers where two records were wanted:
   `rcData has 2 element(s) but its value has 4`. This link was
   probed and named but not built.

Links 1 to 4 were built behind one switch and the corpus run twice
from one binary, giants carved out. The numbers, from
`/tmp/corpus168_off.txt` and `/tmp/corpus168_on.txt`:

```text
off:  1040 examples, 837 flatten, 506 run;  911 runnable, 732 flatten, 474 run
on:   1040 examples, 833 flatten, 506 run;  911 runnable, 728 flatten, 474 run
```

Four models lost to flattening, none gained, and the run lists
identical line for line. Three of the four are the Batteries models
the work was aimed at - `BatteryDischargeCharge`, `CCCV_CellRC`,
`CCCVcharging` - which travelled from the run wall to link five's
flatten wall, which is the chain behaving as mapped. The fourth is
`Modelica.Electrical.Polyphase.Examples.PolyphaseRectifier`, which has
nothing to do with batteries: it flattened before and now refuses with
`dimension of diode1 is not a compile-time constant`. Held back from
overwriting, one of the three guards keeps a length that class needed
to have replaced. Which guard, and why that model wants the later
value, is not known.

So the chain was reverted whole rather than left in. It is not that
the links are wrong - a default overwriting a settled value is a fault
by any reading - but that four links bought nothing, cost a model
outside the family, and the fifth link is where the models actually
stand. Taken again, it should be taken from link five backwards: build
the value correctly and see which of the length guards are then needed
at all.

## Two readings both true: ask where the value came from

Three times in a month the same breed of fault, and each time it read
as a fresh puzzle. A record's constants counted two ways, both right:
a constructor writes no constant, a name written out whole writes them
all, so the value has two lengths and which one arrived says _how the
value was built_ rather than what it means. Before that, a record
table narrowed by whether its key held `].` - a test of who wrote the
entry, dressed as a test of what the name is. Before that again, a
table answering two different questions at two different times.

The rule the three share: when two readings of a value are both true,
the question is not "which is right" but "where did this value come
from". Picking one of the two costs models - nine of them, one shift -
because the answer depends on the writer and the reading cannot see
the writer. Where the structure does not record the origin, the fix is
to make it record the origin, not to find a cleverer test on the
spelling.

This entry exists to be recognised the fourth time rather than solved
again.

## A length a package states is a length, not one that cannot be seen

The whole family of noise blocks - twelve models, the top row of the
flat half's census - stood at `` `state` is given a run of 2
element(s) and 1 value(s) ``. The cause is one line and two storeys
below the message.

`Modelica.Math.Random.Utilities.initialStateWithXorshift64star` fills
a longer state by writing `state[1:2] := aux`, where `aux` is the
answer of `Xorshift64star.initialState`. That function declares
`output Integer state[nState]` against the `constant Integer nState =
2` of the package it sits in. `standing_call` in `arrays.rs` read a
length only where the dimension was a literal number, so a dimension
written as a name answered as the one number a walk always gives - and
a run of two elements was handed one value.

The probe said it in one line, where three shifts of reading would
not have:

```text
PROBE standing: ...Xorshift64star.initialState dims=[Ref("nState")]
```

The constant is looked up in the function's own scope and nowhere
else. A length taken from a name the class does not own would be a
shape said wrongly, and a shape said wrongly here is a shape said
wrongly everywhere below.

### The second link, walked and reverted

Behind it stands `walkable` in `carried.rs`, which refuses a body
answering with several things when one of them is an array of unknown
length. Given the same reading of `nState`, that refusal lifts too,
and the noise blocks then _flatten and run_ - with `state` frozen at
zero and the noise a constant `0.5`. The refusal was not a gap; it was
guarding a real limit of the run, which lays several answers end to
end and has no way to carry the state back out. A wrong number given
quietly is the worst thing this compiler can do, so the second link
was reverted and only the first stands.

That is where the next attempt starts: not at the length, which is
now read, but at the run carrying an array answer back from a body
walked at run time.

## The table that was there and could not be seen

`ModelicaTest.Tables.CombiTable1Ds.Test21` was read as a modifier that
would not travel: `t_new(table=...)` arrives through `extends TestDer`,
and what came out was `the flexible size ':' of 't_new.columns' needs a
value to read its length from`. The inheritance had nothing to do with
it, and the instrument that said so cost a second: `Test20`, which
differs from `Test21` in one word of its file name, ran. So did
`Test23`. Every one of them takes its modifier down the same two
`extends`; only the one naming `test_v7.mat` refused.

What differs between those files is the version of MATLAB that wrote
them. Version 7 writes the same level 5 elements as version 6, each one
deflated and wrapped in an element of type 15, the format's
`miCOMPRESSED`. The reader walked the top level looking for type 14 and
stepped over everything else, so a file with one compressed element in
it read as a file with no matrix at all - and the refusal that reached
the surface was about a length, three layers above the place where the
numbers were not found.

Unpacking is done in front of the walk, so that the walk goes on being
about the level 5 format rather than about how a version stored it: the
header is kept, each compressed element is inflated in place, and what
comes out is a file of the same format nothing downstream has to know
about. A version 6 file pays nothing, because the pass returns `None`
where no element is compressed.

Measured twice from one binary, `--without scripts/heavy_models.txt`,
with the reading held back by an environment switch for the first pass:

```text
/tmp/l170_off.txt  837 flatten, 506 run   (732 / 474 runnable)
/tmp/l170_on.txt   841 flatten, 510 run   (736 / 478 runnable)
```

The diff of both lists names the four and no victims:
`ModelicaTest.Tables.CombiTable1Ds.Test21`,
`ModelicaTest.Tables.CombiTable1Dv.Test21`,
`ModelicaTest.Tables.CombiTimeTable.Test57`, and
`Modelica.Utilities.Examples.ReadRealMatrixFromFile` - the last of
which was never counted as a table model at all and had been reading
its matrix from a compressed file all along.

Three of the seven models that name `test_v7.mat` stand at other walls
and are a separate question: the two `CombiTable2D` tests want
`ModelicaStandardTables_CombiTable2D_getValue`, which is an external
function and not a table format, and `CombiTimeTable.Test81` asks for a
table inside a MATLAB struct, which is a class the reader refuses by
name. Both are honest refusals rather than a silence, which is the
difference this change was about.

## A neighbour's length, and the order it was declared in

The second family of table refusals looks like the first and is not it.
`ModelicaTest.Tables.CombiTimeTable.Test68` writes
`startTime_0(table=startTime.table)` - one component handed the array
parameter of another - and refuses with `the flexible size ':' of
'startTime_0.table' needs a value to read its length from, and
Ref("startTime.table") is not one`. The row held five models before
this shift's change to the MATLAB reader and five after, which is the
register saying plainly that the two families share a wall and not a
cause.

Shrunk to twelve lines, the cause is the order the two were written in:

```modelica
model Fwd
  Modelica.Blocks.Sources.CombiTimeTable later(table=[0,1;1,2;2,3]);
  Modelica.Blocks.Sources.CombiTimeTable earlier(table=later.table);
end Fwd;
```

flattens, and the same two declarations the other way round do not.
`instantiate_components` measures a class's components in the order
they are declared and puts each shape into `sizes_here` as it goes, so
a component asking for a neighbour's length finds it only where the
neighbour was written first. Every one of the five models in the row
writes `startTime_0` above `startTime`, which is the only reason they
are the family they are.

What this wants is a second pass over the components that could not be
measured the first time round, once the ones that could be have put
their shapes in - the same fixpoint the constants layer already runs,
at the shape layer. That is more than the end of a shift, and it is
written down here rather than half-built: the reproduction above is
twelve lines and takes a second, so whoever takes it starts from a
failing model rather than from the corpus.

## An empty array cannot say how wide it is

Seven models refused with `the flexible size ':' of 't_new.columns'
needs a value to read its length from, and Range(Number(2.0), None,
Call("size", [Ref("table"), Number(2.0)])) is not one`
(`/tmp/raw171.txt`, lines 179-198). The names were
`CombiTable1Ds.Test35`, `CombiTable1Dv.Test25_usertab`,
`CombiTable1Dv.Test35`, `CombiTimeTable.Test66_usertab`, `Test80`,
`Test81` and `Test89`.

The mechanism took five lines, and none of them is a table:

```modelica
model H
  parameter Real tab[:, 2] = fill(0.0, 0, 2);
  parameter Integer cols[:] = 2:size(tab, 2);
  Real y = cols[1];
end H;
```

`Value::shape` reads the inner axes off the first element, so a value
with no elements loses every axis but the outer one: `fill(0.0, 0, 2)`
measures as `[0]` rather than `[0, 2]`, `size(tab, 2)` finds no second
axis, and the `:` next door loses its length with it. Every table
block in the standard library declares its table exactly that way,
which is why a fault in the array layer wore a table's clothes.

The declaration is the other witness and it is not empty: `Real
tab[:, 2]` states the two outright, and `shapes.sizes` already holds
it. So where the measurement is short of the axis asked about, the
declaration answers - for the name itself only, never for a member
walked off an array, whose dimensions belong to the array and not to
the member.

That was built, measured, and reverted, and the reversion is the
finding. The numbers first: 841 flatten and 510 run before, 841 and
510 after, and the diff of both lists is empty (`/tmp/l171_off.txt`,
`/tmp/l171_on.txt`, one binary, the fix behind
`OXIDELICA_NO_DECLARED_SIZE`). All seven models moved one storey up
and none of them reached the ground: five now ask for
`ModelicaStandardTables_CombiTimeTable_getValue` or
`_CombiTable1D_getValue`, and the two `usertab` models for their own
`getUsertab` - external C either way, which is a wall this compiler
does not have a door in. `Test87` and `Test88` were never in this
family: they refuse on `t_new.table`, a length taken from a
neighbour, which is the parked fixpoint above.

Then `a_table_file_that_will_not_read_leaves_the_measurement_alone`
went red, and it is right to. A table block whose file cannot be read
declares `table[:, :]` - both axes flexible in the standard library's
own blocks, not the `[:, 2]` of the reproduction - and a declaration
that states a width when the file is missing answers `2` where a
refusal naming the file is owed. A wrong number where a refusal
belongs is the worst thing this compiler does, and the guard that was
already there caught it in seconds.

So the map, for whoever takes it. The cause is certain: `Value::shape`
loses every axis but the outer one when there are no elements, and one
`is_none` in `size` is where it surfaces. What is not settled is who
may answer instead. The declaration may not, because it is silent in
exactly the case that matters and confident in exactly the case that
must refuse. What is wanted is a shape that carries its zero - `[0,
2]` rather than `[0]` - which means `Value` recording the axes of an
empty array rather than deducing them from a first element that does
not exist. That is a change to the representation and not to one
reading of it, and it is written down here rather than half-built.

And the prize is a kind removed rather than a model won: all seven
stand at external C either way. Worth doing for the refusal's sake -
it named an array the model never wrote, and sent three shifts to the
table reader for a fault in the array layer - but not worth a wrong
number bought on the way.

## `an array of shape [2] where a scalar is expected` is a row, not a family

The row counts seven (`/tmp/census171.txt`, line 207) and stands level
with the top of the flat half, so it was scouted. The names, from
`/tmp/raw171.txt` lines 80-172, fall into four unrelated layers:

- `Modelica.Fluid.Examples.ControlledTankSystem.ControlledTanks` and
  `Modelica.StateGraph.Examples.ControlledTanks`, both on
  `start.pre_reset[1]` - a StateGraph array of resets read as one;
- `Modelica.Media.Examples.ReferenceAir.MoistAir1` and
  `ModelicaTest.Media.TestsWithFluid.MediaTestModels.Air.MoistAir`, on
  `specificEnthalpy_pTX` - a medium's mass fractions;
- `Modelica.Fluid.Examples.DrumBoiler.BaseClasses.EquilibriumDrumBoiler`
  on `Medium.dewEnthalpy(sat.psat)`;
- `Modelica.Math.Random.Examples.GenerateRandomNumbers` on
  `Xorshift64star.initialState`, which is the parked noise chain;
- `ModelicaTest.Math.TestNonlinear` on `solveOneNonlinearEquation`.

Two more rows carry the same words with a different shape
(`[1]`, `[3]`, `[6]`, `[100]`), and adding them by wording rather than
by layer would have made a family of nineteen that does not exist.

The medium half shrinks to three lines and does not stay in this row
when it does, which is itself the finding:

```modelica
model M
  package Medium = Modelica.Media.Air.MoistAir;
  Real h = Medium.specificEnthalpy_pTX(101325, 293.15, {0.01});
end M;
```

refuses with `arrays of 1 and 2 elements do not fit together`, and
with `{0.01, 0.99}` it says `arrays of 2 and 3`. The count is off by
one at each try, so a mass-fraction argument is being widened by a
place somewhere between the call and the body - `X` against `Xi`,
which differ by exactly one. That is the layer to take, and it is not
the scalar-where-array reading the row's text advertises. Parked
here with the reproduction rather than half-taken.

## Two records are one shape if only their first field is read

The medium off-by-one scouted above is not the `X` against `Xi`
off-by-one it looked like, and this is the neighbouring wall rather
than the old `nXi` one. The old family stood at a _dimension_ measured
under the wrong scope; this one stands at a _branch_ whose two sides
are records, and nothing about it is medium-specific.

The reproduction is fifteen lines with no medium in it at all:

```modelica
package P
  constant Integer nX = 2;
  record R Real p; Real X[nX]; end R;
  function g
    input Real X[:];
    output R st;
  algorithm
    st := if size(X,1) == nX then R(p=1, X=X)
          else R(p=1, X=cat(1, X, {1 - sum(X)}));
  end g;
end P;
model M P.R st = P.g({0.01}); end M;
```

It refused with `arrays of 1 and 2 elements do not fit together:
Number(0.01) against Number(0.01)` - two arrays the model never wrote,
naming the same number on both sides, which is why three readings took
it for a mass-fraction count.

`Value::shape` reads the depth off the _first_ element. That is right
for an array, whose elements are alike by construction, and wrong for
a record, whose fields travel in the same `Value::Array` and are not
alike at all. Both branches here are `[p, X]`: two fields, first a
scalar - so both shapes come to `[2]`, the branches are judged the
same, and the `if` goes to `zip_values` to be paired element by
element instead of being settled by its condition. The pairing then
meets `X` of one place against `X` of two and refuses about them.

The fix is to compare the whole structure rather than the first
element's, `Value::same_structure` in `flatten/mod.rs`, behind
`OXIDELICA_SHALLOW_BRANCH_SHAPE` for the measurement. The record
branches then differ, the condition is settled - `size({0.01},1)` is
1, `nX` is 2 - and the second branch stands whole.

Measured, both halves from one binary, `--without scripts/heavy_models.txt`:
`/tmp/l172_off.txt` 841 flatten / 510 run, `/tmp/l172_on.txt` 842 / 510.
The diff of both lists is a single line and it is a gain, with no
victim anywhere:

```text
> flat Modelica.Media.Examples.MoistAir
```

One model won and the refusal's family moved: the same `do not fit
together` wording covered thirteen models in `/tmp/raw171.txt` (lines
79-130), twelve of them media. The rest still refuse - `MoistAir`
went one storey up to `an array reached the evaluator`, which is the
next wall and a different one - so this is a kind thinned rather than
a family cleared, and the run count is unmoved at 510 exactly as the
census rule predicts.

### The run half's `unknown variable`, scouted

The register's second run-half family, 18 `unknown variable X in
equation` plus 5 `unknown variable X` (`/tmp/raw171.txt` lines
396-558), is a row and not a family. `--only` against the root `.msl`
puts its members in three unrelated layers:

- **`data` (14)**, on `ModelicaTest.Fluid.TestComponents.Fittings.
TestJunctionIdeal` and its kin: a package's constant record standing
  in a call the compiler left standing - `solveOneNonlinearEquation$...
(200, 6000, data, ...)`. The name arrives in the flat model with no
  prefix at all, which is exactly the invariant AGENTS.md states about
  what survives flattening: a call left standing is named as the flat
  model names it. Nothing declares `data`, so the run cannot find it.
  Its sibling `data.R_s` on `ModelicaTest.Media.TestOnly.DryAirNasa`
  is the same thing a field deep.
- **`ph_explicit` (3)**, on the `TwoPhaseWater` models: a medium's
  `constant Boolean`, named by no equation at all - it reaches the run
  through a `stateSelect` attribute, which is where `Modelica.Media.
Water` writes it (`package.mo` lines 144-188).
- **`liq` (2)**, on `TestSpecificEnthalpy` and `TestSpecificEntropy`:
  a field of the `SaturationProperties` record (`Media/package.mo`
  line 7681), so a record's own field lost between the body that
  writes it and the run.

Three layers, one wording. Working the row's count as one wall would
have aimed fourteen models' worth of effort at a constant-record
question that only fourteen of the twenty-three have. The `data`
half is the one worth taking and it is the largest; parked with the
names here rather than half-taken at the end of a shift.

### The `data` family taken: a chain of five links

The `data` half scouted above was not one wall but five, each behind
the last, all on the same journey: a package's constant record
reaching a call the compiler left standing. The chain was walked to
its end with a twelve-line model before anything was measured, which
is what the notes prescribe for a chain; the corpus was spent once,
at the close.

The model that showed every link, in under a second each time:

```modelica
package P
  record Coeff Real a; Real b; end Coeff;
  constant Coeff data(a = 2.0, b = 3.0);
  partial function ScalarFunction input Real u; output Real y;
    end ScalarFunction;
  function solve input ScalarFunction f; input Real lo; input Real hi;
    output Real r; algorithm r := f(lo) + f(hi); end solve;
  function T_h input Real h; output Real T;
    protected function f_nonlinear extends ScalarFunction;
      input Coeff data; input Real h;
      algorithm y := data.a * u + data.b - h; end f_nonlinear;
    algorithm T := solve(function f_nonlinear(data = data, h = h), 1.0, 2.0);
    end T_h;
  model M Real x; equation x = T_h(0.0); end M;
end P;
```

The links, in the order they were met:

1. **The name was never written out.** `enclosing_constant` answers a
   bare name that is a number and `enclosing_constant_array` one that
   is a list. A record is neither, so both said nothing and `data`
   travelled into the flat model naked -
   `solve$..._f_nonlinear(1, 2, data, 1)` - where nothing declares it.
   `enclosing_record_constant_value` writes it out as the constructor
   its modifiers make, taken from the package's gathering rather than
   from the declaration: an ideal gas declares `constant DataRecord
data` blank and each gas fills it through its `extends`.
2. **The written-out record was spread over the call.** A specialized
   copy is named after what went into it and is not in the registry,
   so every lookup in `expand_call` answered nothing and the call fell
   through to the vectorizing tail. Handed `{2, 3}`, the one call
   became one call per field. The store of specializations already had
   two readers where the registry says nothing; `expand_call` is now
   the third.
3. **The walk's frame held the record and the nested call did not ask
   for it.** `to_scalar` passed a call's arguments down as written, so
   `f.data` - which the frame holds as `f.data[1]`, `f.data[2]` - went
   over as a bare name the evaluation could not look up.
4. **A list argument to a walked call was read as one number.**
   `record_answer` recognised a call answering with several numbers
   and nothing else, so a record written out as `{2, 3}` was taken for
   a scalar. `several_numbers` reads the written-out list too.
5. **The arguments were worked out before the walked-call branch was
   reached.** `eval` evaluated every argument to a number first and
   only then asked whether the name was a body the run carries, so the
   list refused as `an array reached the evaluator` one step short of
   the walk that was going to take it whole. The question is asked
   first now.

One guard went on afterwards and it was a red test that asked for it:
the road is off while a parameter is being settled. A parameter's
value is read field by field by a reader that already exists, and
written out as a constructor instead, a medium's reference enthalpy
came apart into an `s.p` nothing gives a value to. This road is for
the name that survives to the run, where no field reader stands.

**And then the chain was measured, and it is parked.** All five links
work - every small model above runs, and the whole suite is green at
222 - but the first link is too dear and costs a model, which the
corpus said and no small model could:

- The flatten pass did not finish. Unchanged code walks the corpus in
  540s of flatten (`/tmp/base173.txt`, 842 flatten / 510 run); with
  the chain, the pass sat at 936 of 1040 models for forty-eight
  minutes and was killed. The culprit is one model:
  `Modelica.Media.Examples.SolveOneNonlinearEquation.Inverse_sh_TX`,
  over six minutes on its own against about two seconds before.
- `ModelicaTest.Media.TestOnly.DryAirNasa` stops flattening. Under the
  switch it flattens and refuses at `unknown variable data.R_s`; with
  the road on it refuses at flatten instead, for a
  `Functions.thermalConductivity` it cannot resolve. One model given
  back for a family not yet won.

The cause is the shape of link 1 rather than the idea of it. Writing
a record constant out as a constructor puts a whole basket of fields
where a bare name stood, and a medium's field is written as a
property of a state - the dearest road this compiler has. Asked of
every bare name everywhere, that is the cost. Settling the calls in
the constructor was tried and made no difference to either number,
which says the cost is in the substitution the constructor triggers
downstream and not in the folding.

What a next shift would take instead, in the order it should be
tried: write the record out only where the name is about to travel
into a call that is being left standing, rather than for every bare
name the substitution meets. The site that knows this is
`specialized` in `flatten/arrays.rs`, which already has the argument
list in its hands and knows the call will survive. Links 2 through 5
are independent of that choice and stand as written above - each was
seen to be a real wall with a model that reproduced it in under a
second, and each will be met again by whatever road replaces link 1.

### The narrow road through `specialized`, measured and parked

The shift before parked the chain because link 1 - writing a package's
record constant out wherever a bare name was met - cost minutes a
model. The road named as the replacement was to write the record out
only at `specialized` in `flatten/arrays.rs`, where the argument list
is in hand and the call is known to survive. That road was built, the
whole chain with it, and it is parked too, for a different reason than
the first: the cost is not in link 1 at all.

All five links work. The twelve-line model of the section above runs
and gives the right number - `x = 12` against `5 + 7` by hand - and the
suite is green at 221 simulation tests. The links, built where the map
said: `enclosing_record_constant_value` asked from `specialized` alone;
`expand_call` reading the store of specializations; `to_scalar` handing
a nested call the frame's element names rather than the body's bare
one; `several_numbers` beside `record_answer`, so a written-out list is
read as the several numbers it is; and `eval` asking whether the name
is a body the run walks before working its arguments out.

The measurement, both readings from one binary behind
`OXIDELICA_RECORD_CONSTANTS`, on
`Modelica.Media.Examples.SolveOneNonlinearEquation.Inverse_sh_TX`
against the root `.msl`:

```text
road off: 23.8s   (real 0m23.863s)
road on:  killed at 25 minutes
```

And then the switch was moved down one link, which is what the two
builds could not have told apart. With the record-writing off and only
`expand_call`'s reading of the specialization store on, the same model
was killed at eleven minutes. Narrowing that reading further - to a
call that actually carries a written-out array, which is the only case
it was built for - the model was killed again at fifteen. So the dear
thing is link 2: letting a specialized copy stand as a call the run
walks, rather than letting the vectorizing tail take it. That tail is
what was keeping `Inverse_sh_TX` at two seconds, and standing the call
up puts the whole of Brent's method in front of the run instead.

What this says for a next shift, and it is a different question than
the one this chain was walking: the `data` family cannot be won by
making the specialized call stand, because standing it up is what
costs. Either the vectorizing tail has to learn to carry a record
argument whole while still folding what it folds now, or the record
has to reach the run as a declaration of the flat model rather than as
a value in an argument list - a component the flattener writes out,
`data.a` and `data.b`, which is what every other record in a flat
model is. The second is the shape the invariant in AGENTS.md
suggests, and no part of it was tried here.

The chain as built is kept as a patch rather than in the tree
(`/tmp/chain174.patch`, 260 lines): links 3, 4 and 5 are independent of
how the record travels and will be wanted by whatever road replaces
links 1 and 2.

### The road through the flat model's own names, walked to its end

Two shifts parked the `data` family, and both parked on the same
road: writing a package's record constant out as a value that travels
in an argument list. The map left behind named the road not taken -
the record reaching the run as a declaration of the flat model,
`data.a` and `data.b`, which is what every other record in a flat
model is - and said outright that no part of it had been tried. This
is that road, walked to its end.

The base was re-read first, and re-reading it was worth the twenty
seconds. The map of the shift before quoted `Inverse_sh_TX` at "about
two seconds" and its gate at 23.8s, with nothing said about the gap.
On unchanged code, from the binary the gates then used
(`/tmp/gate175a.txt`):

```text
off: real 0m22.855s, and the model does not flatten at all
     ("an equation between shapes [] and [4]")
```

So the 23.8s was never a cost the chain imposed: it is what this model
costs on clean code, and the "two seconds" was a number recalled
rather than read. The notes already warn about that exact failure, and
it had happened again.

The probe then walked the chain with the twelve-line model, and what it
found was not about records travelling as values at all. Written out
by hand, the two readings side by side:

```text
scalar caller: x = P.solve$P_T_h_f_nonlinear(1, 2, d, 0)
record caller: x = P.solve$P_T_h_f_nonlinear(1, 2, data, 0)
```

`d` is the caller's name and `data` is the callee's. A record handed
to a function arrives at `bind_the_arguments` already written out as
its fields, so the branch that takes an `Expr::Array` binds `data.a`
and `data.b` and leaves by `continue` - and the bare name is never
bound to anything. Everywhere a body reads the record field by field
that is exactly right and costs nothing. On the one road where a body
hands a function over, the receiving function is specialized rather
than inlined, its arguments keep the spelling they were written with,
and the unbound bare name travels into the flat model as the callee's
own word for it, which no component of the flat model is called.

Three links, each shown by the small model in under a second:

1. **The bare name is bound too**, to the whole list, so the standing
   call names the caller's record rather than the callee's.
2. **The specialized copy declares the record field by field.** Bound
   whole, the copy had one input where the call now had several
   numbers, and the model was refused for an equation between shapes
   `[]` and `[2]`.
3. **The call the body writes names those fields.** The copy declares
   `f.data.a` and `f.data.b`, so passing `f.data` on was a name the
   run had no value for.

With the three, the small model gives `x = 12` against `5 + 7` by
hand, and the test that asks for that number was seen red without them
(a refusal naming `data` as an unknown variable of an
equation) before it was seen green.

The corpus cost one model, and the one was the finding. Two readings
from one binary behind `OXIDELICA_RECORD_WHOLE`
(`/tmp/corpus175_off.txt`, `/tmp/corpus175_on.txt`): flatten 842 to
841, run 512 to 512, and the diff of both lists named the victim
outright - `ModelicaTest.Media.TestAllProperties.IncompleteMedia.
ReferenceAir_dT`, refused for `an array value cannot be used where a
scalar is expected: Array([Number(1.0)])`. A record of one field is
written out as a list of one, and bound to its bare name it is a list
where the body wanted a number. Binding only where the body actually
hands a function over - the one road the bare name is needed on - gave
the model back to its own older refusal, the unknown function
`density_ps`, which is the wall it stood at before.

The final pass, the switch taken out (`/tmp/corpus175_final.txt`):
842 flatten, 512 run, 737 and 480 runnable - the floors exactly, no
model lost and none won. So this is a wall removed rather than a
model gained, which is the distinction the notes ask to be made
explicitly: `data` no longer reaches a standing call under the
callee's spelling, and the `data` family's models still stand at
whatever was behind that. The gate model `Inverse_sh_TX` cost 22.9s
with the road on against 22.9s with it off, against a kill threshold
named before the measurement at four minutes - which is the other
half of the finding, since the road these shifts parked cost
twenty-five minutes on that same model.

### What the census says the `data` family now is

The census after the road above (`/tmp/census175.txt`, raw in
`~/.jcode/scratch/refusals-raw.txt:556`) counts the family at 14 models
refusing as an unknown variable `data` plus one on `data.R_s` - the
same wording as before, and the number is honest rather than
disappointing, because the compiler's own probe says these are a
different shape of the same wall:

```text
in Modelica.Media.Examples.SolveOneNonlinearEquation.Inverse_sh_T:
  declared: nowhere - no component of the flat model is called that
  equation: Th = ...T_h_f_nonlinear(200, 6000, data, h1)
```

In the small model the caller had the record as a component - `d.a`
and `d.b` were there to be named, and what was missing was only the
binding of the bare name to them. Here there is no caller component at
all: `data` is a constant of the gas package, and nothing in the flat
model declares it under any name. So the road taken removes the wall
for a record a model holds, and the standard library's own case wants
the second half of the same invariant - the package's constant
declared as a component of the flat model, the way `minted_constants`
already declares a medium's scalar constant that could not be folded.
That is the next link, and the mechanism for it is in the tree
already, which is what the two parked attempts did not have.

### The named road, and what it was worth

The probe went looking for the package constant declared as a
component of the flat model, and found the bottom a link short of it.
`ModelicaTest.Media.TestOnly.DryAirNasa` refused on `data.R_s`, and
the smallest model that refuses the same way is five lines: a gas
package whose function reads a field of a record it was handed. What
`bind_the_arguments` does with an argument depends on how the input
was found rather than on what the argument means - a record given in
order was resolved to the constant it names and bound field by field,
and a record given by name was bound whole, as the bare name alone.
`Functions.h_T(data = data, T = u)` is the standard library's own
spelling, so the whole of the ideal-gas chain travelled the second
road. The two branches now share `record_argument` and
`bind_record_argument`; the difference between them was never
intended, it was what a `match` on the argument's shape happened to
leave out.

Measured twice from one binary, `OXIDELICA_NO_NAMED_RECORDS=1` for the
old road: 842 flatten and 512 run either way
(`/tmp/p176/corpus.off.txt`, `/tmp/p176/corpus.on.txt`, both reading
`of the 842 that flatten, 512 run`), and the run lists identical model
for model. What moved is the register: the row for an unknown
variable `data.R_s` in an equation (1) is gone, and `structurally singular model:
cannot differentiate this expre` went 9 to 10. One model, one storey
up - a kind removed rather than a model won, which is the commoner of
the two outcomes and worth recording as what it is.

Where the small model would not serve, and why it took three tries: a
scalar field of a constant record folds on the constants road whichever
way it was handed over, so a synthetic model reading `data.R_s` runs
on both sides of the switch and witnesses nothing. Only a subscripted
field waits for the binding. That is the test, and it was seen red
with the switch on before it was seen green with it off.

The family is where it was: 14 models on the bare `data`, which is the
constant of the gas package with no component behind it at all, and
that is still the next link - the road this shift took was the one
below it. The probe for it is `/tmp/p176/X.mo`, five lines that refuse
for an unknown variable `data` in an equation, exactly as the fourteen
do.

## Shift 177: the `data` chain walked to its end

Three shifts parked this family and each parked with a map. The map
was right about the wall and wrong about its depth: what looked like
one link - a record constant handed over under its bare name - was
eight, each one behind the last, and every one of them invisible until
the one before it fell. This is the shape the notes describe as a
chain, and the rule they give is the one that worked: walk it to the
end with the probe first, keeping each removal local, and only then
take it as one series.

The links, in the order they were met, each with the small model that
showed it:

1. **A record handed over under its bare name is not read at all.**
   `function f_nonlinear(data = data)` appends the name `data` to a
   specialized copy that declares fields, so the flat model was
   refused for a variable no model wrote. The splitting existed and
   fired only for a value already written out as an `Expr::Array`.
   (`A1.mo`, 30 lines, no standard library.)
2. **The fields are read where the call was written.** Split, they
   became `data.R_s` and the like, which mean nothing further along.
3. **A field that is text is not a number.** A medium's record carries
   its own `name` beside its gas constants; handed over as a number it
   becomes a name nothing declares. `Inverse_sh_T` died on `data.name`
   at exactly this link.
4. **A carried body does not read its package's bare constants.** A
   body too large to inline is walked, and it names `reference_p` with
   no path; the dotted road never saw it because the name has no dot.
   (`C3.mo` - and `C1.mo` beside it, the same body small enough to
   inline, which runs. The pair is what located the layer.)
5. **The walk binds by position and a named argument does not know
   it.** `Functions.h_T(data = data, T = u)` handed the walk `data`
   where `T` was declared. This one is worse than a refusal: `F2.mo`
   answered **300000** before the fix and **200.1** after, and 200.1
   is the number worked out by hand. A wrong number had been sitting
   in the walk for every carried body that used a keyword.
6. **A record with an array field was passed over whole.** The NASA
   gas record is fourteen coefficients against four scalars, and a
   splitting that could only do scalars gave up on all of it.
7. **A subscripted field is one flat name, not a subscript of one.**
   The renaming speaks for `data.alow[1]`; the body writes an index of
   `data.alow`, which the map has never heard of.
8. **A negative number is not a literal.** Only the odd coefficients
   folded: `Expr::Neg(Number)` is not `Expr::Number`, and every even
   NASA coefficient is negative. This is why the refusal named
   `data.alow[2]` and never `alow[1]`.

`Inverse_sh_T` now flattens and runs to the standard library's own
`solveOneNonlinearEquation`, where it meets the parked `String(x)`
wall - a different family, and the honest end of this one.

The fourteen small models are kept in `~/p177`; each gives a number
checked by hand rather than merely flattening, which is what the notes
ask for and what caught link 5. Link 5 is also the answer to why three
shifts found nothing: every earlier synthetic model passed scalars
positionally, and nothing about a scalar passed positionally is wrong.

## A body called only inside a branch the run decides

The census after the `data` line closed put seven models on one row of
the run half: `unknown function Modelica.Fluid.Utilities.regSquare2`,
a name whose text the compiler was carrying at that very moment
(`~/p178/census.txt`, run half).

The probe took under a minute once the ladder was climbed in order. A
small model calling `regSquare2` from an equation runs and gives 0.25
by hand; from an array; from under a `smooth`. All green. What is
different about a vessel is where the call sits: `Vessels.mo:360`
writes the port pressure inside `if regularFlow[i] then`, and
`regularFlow` is a variable, so the branch is one the run picks.

Such an `if` is not among the flat model's equations. It is held apart
in `Model::conditional`, one list per branch, precisely because which
branch holds is unknown until the run. And `programs_used`, which
gathers the bodies a model must carry into the run, walked the
equations, the initial equations, the asserts and the `when` clauses -
not the conditional. A function reached only from inside such a branch
travelled with nothing, and the run met a name it had no body for.

The smallest model that shows it is twelve lines, and it was seen red:
a `while` body called under `if big then`, where `big` is a Boolean
equation. Reproduced in one second what the corpus says in eleven
minutes.

Measured twice from one binary, without the carved-out giants
(`~/p178/corpus.on.txt:176`, `corpus.off.txt:176`): 842 flatten and
512 run become 842 and 513. The flatten list is identical line for
line; the run list gains
`ModelicaTest.Fluid.TestComponents.Vessels.TestVolume` and loses
nothing. The other six of the row are the expected shape - a barrier
removed uncovers the next one, and `ThreeTanks` now stops on an
algebraic loop rather than on a name.

So one model on the count, and a row of seven emptied: the two are
separate claims, as the notes require, and this change is both.

## A steady start written about a variable the plan computes

The census after the `regSquare2` line closed put seventeen models of
the run half on one wording: `volume.medium.T` is not a state of the
model, and its relatives naming `pump.medium.h`,
`pipe.mediums[1].h` and `aimc.airGap.V_msr.re`
(`/tmp/p179-census.txt:271` and following, read by name from
`/tmp/p179-raw.txt`). The counter split the family across several rows
because it quotes whichever name came first; added up, seventeen.

`oxidelica why` on one of them answered the question in a second. The
medium declares `T` with no binding, an equation writes
`h = cp_const*(T - 298.15)` and the volume around it holds `h`, so
`T` is an algebraic variable of the plan rather than a state - and the
model's `initial equation der(volume.medium.T) = 0` names it.
`substitute_derivatives` knew only the states, so it refused.

The compiler had what it needed in hand. Everything that differentiates
a definition already exists for index reduction, and the plan's own
explicit stages are the definitions. So the initialisation asks for
them: `der` of a name the plan assigns explicitly becomes the chain
rule applied to its definition, and `der(h) = cp*der(T)` with the
states' derivatives already known.

Nothing is guessed. A name a simultaneous block solves for has no
definition to read and is refused exactly as before - `b + sin(b) = a`
determines `b` and no rearrangement gets it alone on a side - and so is
a definition this module cannot differentiate, and so is a derivative
that would have to be minted, since the initialisation has no equation
to bring for one. That is the test that stands where the old refusal
case stood.

The smallest model is seven lines and was seen red behind the switch:
`der(u) = -u; h = 2*u; w = h - 3` with `initial equation der(w) = 0`,
which puts `u` at zero and `w` at -3. The number is what the test
holds, not the fact of running.

Measured twice from one binary, without the carved-out giants
(`/tmp/p179-on.txt:176`, `/tmp/p179-off.txt:176`): 842 flatten and 513
run become 842 and 515; the runnable pair 737/481 becomes 737/483. The
flatten list is identical line for line and the run list loses nothing.
The two gained are `Modelica.Blocks.Examples.InverseModel` and
`ModelicaTest.Media.TestsWithFluid.MediaTestModels.Water.ConstantPropertyLiquidWater`.

So two on the count and a row of seventeen emptied, and those are the
separate claims the notes ask for. Behind the wall stands another for
most of the family: the media tests that anchor a temperature reach
their initialisation and meet a solver there instead, which is the
expected shape of a barrier removed rather than a disappointment.

## A name declared by two bases at once

The `unbalanced` row of the run census counted 48 models
(`/tmp/census180.txt:275` and following), and the machine-shaped part
of it had been probed before. The rest had not, and the register's
wording splits it: five models were refused with `nothing determines
simpleGenericOrifice.m_flow`, three more named an orifice or an
expansion by another name, and the fluid sensors and fittings between
them make a family the count never showed as one.

The probe found the same shape in all of them. `oxidelica why
ModelicaTest.Fluid.TestComponents.Sensors.TestPressure
simpleGenericOrifice.m_flow` prints the declaration twice over, and
the two are not the same declaration: `PartialTwoPortTransport`
declares the mass flow with a state selection written on it and
`PartialLumpedFlow` declares it plainly, and every fitting of the
library extends both. The flat model has one place for a name, so the
second declaration was a second unknown standing against the same
equations - the count one short, and the refusal naming the copy.

The rule that was already there could not see it. It asks whether a
base repeats a declaration word for word, which these two do not, and
it asks it of the class text, where the flat name does not exist yet.
What settles the question is the flat name: a primitive whose name has
already been written out is a repetition whatever words brought it.

The smallest model is nine lines and was seen red behind the switch:
two bases declaring `w`, one with a `start` and one without, a class
extending both, and `w = 2*time` with `y = 3*w`. The test holds the
numbers, 2 and 6, rather than the fact that it ran.

Measured twice from one binary, without the carved-out giants
(`/tmp/p180-off.txt:1043`, `/tmp/p180-on.txt:1043`): 842 flatten and
515 run become 842 and 516; the runnable pair 737/483 becomes 737/484.
The flatten list is identical line for line and the run list loses
nothing; the one gained is
`ModelicaTest.Fluid.TestComponents.Fittings.TestSimpleGenericOrifice`.

One on the count, and a family of eight taken off the wall it stood
at: the sensors and the remaining fittings reach their solver instead,
which is the expected shape of a barrier removed rather than a
disappointment.

## A name two bases bind differently

The rule of the shift before took a name two bases both declare as the
one element the flat model has a place for, and dropped the second
declaration outright. For the mass flow of the fluid library that is
right - neither base binds it - but a dropped declaration takes its
declaration equation with it, and the rule said nothing about a
declaration that brought a value.

Two twelve-line models settle what happened. `model A Real w = 2.0`
and `model B Real w = 3.0`, extended together, printed `w = 2` and
`y = 2` without a word: the first base won a contest nobody was told
about, which is a wrong number presented as a right one. And where
only the second base binds - `A Real w; B Real w = 3.0` - the binding
vanished with the declaration and the model was refused as unbalanced
about `y`, a name that had nothing to do with it.

What settles a repetition is not the name alone but what the two
declarations say about the value. Both silent is the fluid library's
case, and the second falls away as before. Both binding the same value
is a repetition too, compared by value rather than by spelling, since
`2` and `2.0` are one number and a refusal about the spelling would be
a test on the text standing in for a test on the value. Where they
differ there is nothing to choose between them, and the compiler
refuses, naming both bindings.

Measured over the library without the carved-out giants, the refusal
fires nowhere: the standard library has no pair of bases that bind one
name to two values, so the flatten and run lists are identical line for
line with the switch `OXIDELICA_TAKE_FIRST_OF_TWO` on and off. What the
change buys is not models but the absence of a silent wrong number.

## The unbalanced remainder is one family, not two

The run census at 37 unbalanced models (`/tmp/census181.txt:271` and
following, down from 48 in `/tmp/census180.txt`) shows two addresses
that read as separate barriers: six `AST_BatchPlant` models refused
about `tank.portsData_height[1] = tank.portsData_height2[1]`, and four
`StateGraph` composite steps about `initStep.inPort[1].occupied =
inPort.occupied`. Probed, they are one family.

`oxidelica why ... inPort.occupied` prints the shape whole: the
connector's member is settled once by `outPort.available = false` and
again by the connection equation, so the model carries one equation
more than it has unknowns and the matching hands back the connection
as the one thing left over. The tank is the same with a conditional
declaration in the way - `portsData_height` is a `RealInput` connected
to an internal one that exists only `if use_portsData`, and both the
`if not use_portsData` branch and the connection write it.

So the barrier is not two addresses but one: a connection equation
written where the connector's value is already determined. That is a
question about how conditional components and connection equations are
counted together, and it is left where it is rather than taken in the
same shift as a refusal about declarations - named here so the next
shift does not rediscover it as two.

## A connector is recognised by its name, and a name may be imported

The `Electrical.Digital` family stood twenty models deep at

```text
connect(s, Nor1.x[2]): both sides must be connector instances
```

which reads as a question about subscripts and about arrays of
connectors, and is neither. Probed, `connect(s, q)` of two plain
scalar ports of that library was refused in exactly the same words -
the array was innocent, and the subscript with it.

What the layer actually is: a connector written as one value rather
than as a set of members - `connector DigitalInput = input
DigitalSignal` - cannot be recognised by its type, because resolving
the type leaves the primitive behind and a primitive says nothing
about being connectable. So it is recognised by its _name_, in
`names_a_connector` (`crates/oxidelica-parser/src/flatten/lookup.rs`),
which splits a dotted name and asks the holder whether it wrote a
short `connector` definition of that name. The holder was looked up
with `plain_lookup`, the walk out of the enclosing packages, which
knows nothing of the imports. Every model of `Electrical.Digital` is
written with `import D = Modelica.Electrical.Digital` throughout, so
the holder came back unfound, the declaration was not a connector, it
never entered the table of connector instances, and the `connect` was
refused as naming something that is not one.

The tell was there to be read before any of this: the same two
declarations written out in full worked, and written through the
import did not. A name that means the same thing gave two answers
depending on who spelled it.

Measured over one binary behind `OXIDELICA_NO_IMPORT_HOLDER`, with
`--without scripts/heavy_models.txt`: flatten 842 to 854
(`/tmp/before183.txt` and `/tmp/after183.txt`, line "classes:"),
runnable flatten 737 to 742, run 516 either way. The diff of the two
`flat` lists names twelve arrivals and no victims - eleven
`Electrical.Digital` models and `Analog.Examples.AD_DA_conversion`,
which is written the same way.

The run count did not move, and that is the honest half of the report:
the family stands one storey up now. `Utilities.RS` flattens and meets
`delay(..., 0)`; `BUF3S` meets an array of shape [4] where a scalar is
expected; `DFFREG` meets a `break` whose condition the compiler cannot
decide. Those are three different walls in three different layers, not
a chain, which is why they were not taken in one series. The census
after the change will show them as three rows rather than one, and the
row this change emptied - twenty-odd `both sides must be connector
instances` in `/tmp/on183.txt` - is a kind removed rather than models
won.

Two things the probe ruled out, so the next shift does not walk them
again. The family is _not_ the parked one of map 181 (a connection
equation written where the connector's value is already determined -
six `AST_BatchPlant` tanks and four `StateGraph` steps): that is a
counting question about conditional components, and this was a lookup
that could not see an import. And the `delay(..., 0)` that `RS` meets
is an artefact of probing a helper directly - `RS` defaults
`delayTime` to zero and its real callers give it a value.

The five refusals still reading "the subscript of `X` must be a whole
number the compiler can see" beyond the two FFT ones, named because a
count without names is a family nobody can plan against
(`~/.jcode/scratch/refusals-raw.txt`, lines 19, 20, 90, 91, 142, 146,
147): `Blocks.Examples.Rectifier12pulseFFT`,
`Blocks.Examples.Rectifier6pulseFFT`,
`Math.FastFourierTransform.Examples.RealFFT1` and `RealFFT2`,
`ModelicaTest.Math.TestMatrices2b`, `TestPolynomials` and
`TestVectors`. Seven, not five: the count of the row is seven and the
two FFT examples are inside it, which is the rows-that-mean-the-same
trap read from the other side.

## The subscript register is three layers, not one

Forty models refused around the words "the compiler cannot see this
subscript", which reads as one wall and is three.

The first is a table written in a package and read by a subscript the
run settles: the digital gates ask `Tables.AndTable[auxiliary[i],
x[i + 1]]` of signals. A table written inside the model was already
answered - the array layer asks every place in turn and builds a chain
of `if index == k then a[k]` - and the package one was not, because the
constant lookup took a list written with the matrix brackets for
something other than a list and handed back nothing. The name then
travelled with an instance path stuck on the front,
`Nor1.Modelica.Electrical.Digital.Tables.OrTable`, which is the
aggregate fault this document already names from three other ends. The
door is that the brackets an author reached for are not a fact about
the value, and reading `[1,2; 3,4]` as a list closes it: twenty
subscript refusals across `Modelica.Electrical.Digital` are gone, and
the family now stands one storey up, at `connect` over a bus element
and at a `break` whose condition the compiler cannot decide.

The second is `the subscript of X must be a whole number the compiler
can see`, seven models, the FFT buffers and the vector tests. That is a
statement layer and not this one - an assignment's target rather than
an expression's base - and it is untouched by the above.

The third is `a slicing subscript must be constant at compile time`,
six models around `R134a`: a subscript that is a vector of places
rather than one, where the nested-`if` answer does not apply because
what is asked for is several elements at once. A third layer again.

So the register's forty was a lower bound on families, exactly as the
counting note says, and the probe put them in three. Only the first was
a door.

## What a second base says about a name it shares

A name two bases both declare is now refused when they bind it to two
different values. What neither base's _modifications_ say was measured
this shift and is recorded here unbuilt.

`A` declaring `Real w(start = 1)` and `B` declaring `Real w(start = 2)`,
with `D` extending both, comes out with `start = 1`; swap the order of
the two `extends` clauses and it comes out with `start = 2`. The same
for `fixed`, which is a number in the answer rather than a hint at one.
So the winner is whichever clause was written first, and nobody says a
word about the loser.

That is the same silence the binding case was refused for, one attribute
down. It is not built because the fluid libraries legitimately write
`start` on one base and leave it off the other, and a refusal at any
disagreement would take them out: the only honest refusal here is over
two attributes both _written_ and _different_, which is a narrower test
than the one the binding case needed and wants its own measurement.

## What a bracket around a deep array lost, and what the fall-through typed

The tristate half of `Modelica.Electrical.Digital` stood at `an array
of shape [4] is used where a scalar is expected`, four models -
`BUF3S`, `INV3S`, `MUX2x1`, `WiredX`. The probe put it in the array
layer and not in the subscript one: `Buf3sTable[S, R, R]` is written
`[{{{...}}}]`, one array of three dimensions inside the matrix
brackets, and the matrix reading joins rows side by side into cells
that are scalars. The third dimension has nowhere to go, so what came
back for `Table[strength, enable, input]` was a row of four.

Which bracket an author reached for is not a fact about the value -
the same door the last shift opened from the other side. A matrix
bracket holding a single part, and that part already an array of three
dimensions or more, is a list somebody wrapped: it is handed over
whole.

Behind it stood a second wall in a different layer, and the two are
one chain rather than two findings. Reading an array by a subscript
the run settles builds `if index == k then a[k]`, and the chain ends
in the compiler's own `NaN` - the value of an index outside the array.
The type layer read that `NaN` as a Real, because it is a number whose
fractional part is not zero, so the whole chain came out Real and the
Integer target of `lh := delayTable[y_old, x]` was told it was being
handed a Real. A refusal about a number nobody wrote. What has no
value has no type, so the fall-through answers `Unknown` and the chain
keeps the type of the places that were actually written.

The chain ends there, in a different family: all four models now
flatten and refuse as unbalanced, with nothing determining the
connectors of the component under test - `bUF3S.enable`, `bUF3S.x`,
`bUF3S.y`. That is the connector-side wall the map already carries and
not this layer, so the chain was walked to its end and stopped at the
boundary.

The `break` wall of `DFFREG` was probed the same way and is not a
door: with every `break` taken out of the source by hand the model
dies one step later, at `` `nextstate[1]` is assigned in one branch
only and has no value before the `if` `` - the merge in
`statements.rs` refusing to give an array-writing target a start.
Eight Digital models stand there. Parked with the layer named.

## A slot the run fills has to exist before a switch names it

`$initial` was given its slot ahead of the discrete definitions
because a discrete variable may be defined in terms of it, and
compiling that definition reads the slot before the line that would
have made it. The same is true of three more names the compiler mints
for itself - `$delay`, `$sample`, and a connector's transport - and
those were still being made after the definitions.

A flip-flop is exactly the shape that trips it: a delayed signal
compared against a threshold is a Boolean whose definition reads
`$delay0`. Seven lines reproduce it, `b = delay(u, 0.1) > 0.5`, and
what comes back is ``unknown variable `$delay0` `` - the compiler's own
name, which is the tell that nothing in the model is at fault.
Ten models of `Modelica.Electrical.Digital` stood there, the whole
``unknown variable `$delay0` `` row of the census at 444958e - the
row reads `10` on line 551 of `refusals_raw_444958e.txt`, and the
names on lines 250 to 267 are `Examples.Counter`, `Examples.Counter3`,
`Examples.FlipFlop`, `Examples.FullAdder`, `Examples.Multiplexer`,
`Examples.Utilities.Counter`, `Examples.Utilities.Counter3`,
`Examples.Utilities.DFF`, `Examples.Utilities.JKFF` and
`Examples.Utilities.RSFF`. Two pairs of namesakes sit in that list,
`Counter` and `Counter3` under both `Examples` and
`Examples.Utilities`, which is how the count was first read as nine.

The row is now empty and the run count did not move: all ten travel
one storey up, to `the event at t = 0 does not come to rest after N
round(s)` - `DFF` at ten rounds, `Counter` at eighty. That is a
different layer, the event iteration rather than the slot table, and
it is where the Digital family now waits. A kind removed is worth
recording as a kind removed; the models behind it had a second wall.

## A switch resting on its threshold was lost for the rest of the run

`AD_DA_conversion` was the one model in the library to meet the event
ceiling - ten thousand events inside a window of 10^-8 seconds, at
`refusals_raw_444958e.txt` line 218 - and the working guess was that
the chatter was ours rather than the model's. It is, and the small
model that shows it is four lines:

```modelica
model S Real x(start = 0, fixed = true); Boolean on;
equation on = x > 0.5; der(x) = if on then -1 else 1; end S;
```

A sliding mode: whichever side of the threshold `x` stands on it is
driven back, so the event settles exactly on `0.5` and the next step
carries the state off it. The indicator therefore reads zero where the
step begins and has turned where it ends, and `dopri.rs` dropped that
crossing outright whenever the instant was one an event had just been
handled at. That instant is every step of a sliding mode, so the
switch kept the value it had and kept it for good: measured on the
binary at 50d9238, `x` walked out to `2` at the stop time with `on`
reading false the whole way, while its own equation says `on = x >
0.5`. A wrong number presented as a right one, which is the worst
thing this compiler can do, and the guard was there to stop the run
standing still rather than to hide a switch.

Standing still and dropping the crossing are not the same choice.
The event is now raised at the far end of the step rather than at the
instant: the run advances by a whole step, which is all the guard was
protecting, and the switch is tested where it has actually turned. The
sliding model then refuses, naming the chatter and the window, which
is the honest answer - a sliding mode has no solution in this
formulation and the ceiling built for `AD_DA_conversion` is what says
so.

Measured with `scripts/library_floor.sh .msl` on one binary either
side: 868 flatten and 519 run, runnable 753 and 487 - the floors to
the digit, unmoved. A wrong answer removed for nothing.

### What the same loop layer would not give

The top of the run census is the algebraic-loop layer, 71 models by
the count of the five rows at `refusals_raw_444958e.txt` (30 NaN
before any Newton step, 14, 10, 9, 8). Its smallest member,
`Modelica.Magnetic.FluxTubes.Examples.BasicExamples.SaturatedInductor`,
reduces to a loop of four unknowns around `R_m = 1/G_m`, and a
twelve-line model reproduces the refusal in a second.

The suspicion was the singularity test: a magnetic permeance near
1e-9 and the reluctance that is its reciprocal near 1e9 put a column
of 1e18 beside one of 1e-18 in a Jacobian whose rows are scaled and
whose columns are not, and the small column then reads as the
finite-difference noise the test exists to catch. Scaling each column
by the step its own unknown takes was built behind
`OXIDELICA_NO_COLUMN_SCALING` and measured on one binary: the small
model travels from `underdetermined` to `did not converge`, and the
corpus falls to 516 run against a floor of 519 - three models paid for
a barrier that did not move, because the MSL model's own refusal is
raised before Newton takes a step and no scaling of the Jacobian can
reach it. Reverted. The test that caught the first version of it is
worth recording too: without a guard for the one-unknown block,
`1 / x = 0` came back with a number instead of a refusal.

## Where a torn block takes its first values from

The refusal that stops thirty models is raised before Newton takes a
step, so the question the previous shift left was the one about the
values the block is handed rather than about the iteration. Probed
directly - the starting vector printed at the head of
`solve_implicit_block_from` - the answer is that nothing is broken
there. A declared `start` reaches the torn unknown: a two-equation
loop over `a(start = 7)` and `b(start = 5)` starts at `b=5`, and the
same model with the attribute left off starts at zero. The seeding is
honest, and the zeros are what the library wrote.

`NonLinearInductor` shows both halves of it. Its block starts at
`["r_mFe.mu_r=0", "r_mFe.R_m=0", "r_mFe.Phi=0", ...]` - twelve
unknowns, every one of them zero, because not one of the twelve
carries a `start` attribute. `mu_r` is an `SI.RelativePermeability`,
which is `Real (final quantity=..., final unit="1")` and says nothing
about where the variable begins; the declaration in `FixedShape` is
the bare `SI.RelativePermeability mu_r`. So the zero is not a failure
of the compiler to read an attribute. It is the absence of any
attribute to read, and the value a Modelica variable takes without one
is zero by the specification.

The retry from a handful of magnitudes, built last week, is what the
model now lives on, and the probe says which magnitude: the inductor
is refused at 1e-6, 1e-3 and 1.0 for a singular Jacobian and solved at
1e3 - and at the next call the same 1e3 comes back
`underdetermined`. So the block's scale is above the ladder's top
rung at one point of the run and the ladder is a guess at what only
the model knows.

What the library has and the compiler does not read is `nominal`,
which is the attribute written for exactly this purpose: a magnitude
for a variable that has no start. The parser reaches it and throws it
away - `declarations.rs` puts it in the arm that parses the remaining
attributes and drops them, beside `quantity` and `stateSelect`. That
is a policy choice about initial guesses rather than a defect, and it
is parked here rather than taken: reading `nominal` into the starting
vector changes where every torn block in the library begins, and the
measurement it wants is the diff of the run list rather than the
floors.

## The instrument that could not see a binding

`why` read the equations of the flat model and the `when` clauses, and
not the bindings of the declarations - so a name used only by another
declaration's binding came back as `named by: no equation of the flat
model`. That is the whole population of record parameters:
`battery.resistor[1].R = (battery.Ns * battery.cellData.rcData.R) /
battery.Np` is a binding, not an equation, and asking `why` about
`battery.cellData.rcData.R` - the name the compiler was at that moment
refusing to evaluate - answered that nothing in the model mentions it.
An instrument that says a name does not exist while the refusal beside
it quotes that name sends the reader after a flattening fault that is
not there.

The reproduction is four lines: a record with one unbound parameter, a
second parameter bound to it, and an equation using the second. The
fix reads the components' bindings alongside the equations and prints
`binding: r = d.R`.

## CCCV_Cell counts one state twice

`initialization is not square: 4 initial equation(s) and 1 fixed
start(s) for 3 unknown(s)`. Probed, the four are two and two: two
written initial equations (`cell.limIntegrator.y =
cell.limIntegrator.y_start`, `energy.y = energy.y_start`) and two
entries of `fixed_starts`, the conditions carried by variables index
reduction demoted - `cell.cell.SOC = 0.1` and `cccvCharger.CV = 0`.

`SOC` is declared `output Real SOC(start=cellData.SOCmax) =
limIntegrator.y`, an alias of the integrator's output. So its `fixed`
condition and `cell.cell.limIntegrator.y`, which the probe shows
already pinned, are one statement counted in two places: once as a
condition and once as a pinned start. `cccvCharger.CV` is the other
kind - a condition on a variable that is not among the three unknowns
at all. Both are the aggregate fault the notes already name, a value
read two ways because the reading cannot see which writer produced
it. The work is to make `fixed_starts` say whether the demoted
variable is an alias of something already pinned, and that is a change
to what the structure records rather than a test on the spelling.

### What the matching answered, and what it did not

Both kinds turned out to be one question rather than two, and the
question is not what a condition is spelled but what it determines.
The written `initial equation` section was already matched against the
states through the plan; the demoted `fixed` declarations were counted
beside that matching without taking part in it. Put into the same
matching - each demoted variable reaching whatever states its own
definition reaches - an alias finds no state of its own, because the
one behind it is already claimed by the equation that settles it, and
`cccvCharger.CV` finds none because it reaches none. Neither needs a
name read.

Measured from one binary, the fix behind `OXIDELICA_COUNT_EVERY_CONDITION`:
the `initialization is not square` row went 24 to 19
(`/tmp/m191/before.txt:252`, `/tmp/m191/after.txt:252`). That is not a
net five leaving: the row is 24 - 15 + 10, and the names come from
diffing the two raw lists rather than from the count.

Fifteen left: `CCCV_Cell` and `CCCV_CellRC`; the `Machines` induction
machines `IMC_DOL`, `IMC_Inverter`, `IMC_Steinmetz`, `IMC_Transformer`,
`IMC_YD`, `IMC_YDarc` and `IMS_Start`, with
`Transformers.IMC_Transformer` beside them; `FundamentalWave`'s
`IMC_DOL` and `IMS_Start`; and the three `WaterIF97` media tests
`WaterIF97OnePhase_ph`, `WaterIF97_pT` and `WaterIF97_ph`.

Ten arrived, all synchronous machines: `Machines`' `SMEE_Rectifier`,
`SMPM_Braking`, `SMPM_Inverter`, `SMPM_VoltageSource`, `SMR_DOL` and
`SMR_Inverter`; `FundamentalWave`'s `SMEE_Rectifier`, `SMPM_Inverter`
and `SMR_Inverter`; and `QuasiStatic`'s `SMPM_Mains`. They came up one
storey from `airGap.RotationMatrix[1,1] ... is NaN at t = 0`, which is
the family the register was already counting.

The three `WaterIF97` tests went somewhere else again, and that is the
part a retelling lost: all three now stop at a wall that did not exist
for them before, `der(volume.medium.h)` or `der(volume.medium.T)`
reporting that the differentiated variable `is not a state of the
model` (`/tmp/m191/raw_after.txt`). The register shows the three
arriving across two rows rather than one: `der(volume.medium.T)` goes
from 5 to 6 and `der(volume.medium.h)` from absent to 2
(`/tmp/m191/before.txt:261`, `/tmp/m191/after.txt:261,285`). Counting
either row alone would have found one model where three moved, which
is the row-is-not-a-family blind spot seen from inside a single
change.

The set of refused models is identical line for line, 520 either way
(`/tmp/m191/raw_before.txt`, `/tmp/m191/raw_after.txt`), and the floors
did not move: 868 flatten and 520 run, runnable 753 and 488
(`/tmp/m191/floor_after.txt`). So this is a wall removed and not a
model won - behind the double count of the batteries stands whatever
their initialisation actually cannot solve, and behind the synchronous
machines stands the NaN in the air gap.

### What now stands at `not square`, named rather than counted

The count said the row held nineteen and said nothing about which
statement in each was the extra one, so the refusal was taught to
print the parts of its own arithmetic: how many equations the model
wrote, which demoted starts were counted beside them, and which states
stayed pinned. That is one command per model now instead of a guess.

The hypothesis offered for the surplus was that it is a
two-component quantity counted twice - a `SpacePhasor` array of two or
a `re`/`im` pair - which would make every surplus even and a multiple
of two. It is dead, and the first model killed it. `SMPM_Braking`:

```text
4 initial equation(s) and 3 fixed start(s) for 5 unknown(s);
the conditions are 3 written equation(s) and the demoted start(s)
[smpm.wMechanical], the pinned states are
[smpm.airGap.psi_ms[1], smpm.airGap.psi_mr[1], smpm.airGap.psi_mr[2]]
```

The surplus of two is not two components of one object. It is one
scalar mechanical speed counted as a condition while three separate
flux states stay pinned, and `psi_ms[1]` and `psi_mr[1]` are not two
halves of anything - they are the stator's and the rotor's.

Measured on all ten arrivals from one binary, the shape that does
repeat is a different one and it is a shape about the writer, not
about the arity. Two writers supply conditions here: the model's own
`initial equation` section, and the `fixed = true` declarations index
reduction demoted. Across the ten, the demoted list is almost entirely
three names - `wMechanical`, `ir[2]` and, once, `gamma`/`gammar`:

```text
Machines.SMPM_Braking        3 written + [wMechanical]
Machines.SMR_DOL             2 written + [wMechanical, ir[2]]
Machines.SMPM_Inverter       2 written + [wMechanical, ir[2]]
Machines.SMR_Inverter        2 written + [wMechanical, ir[2]]
Machines.SMPM_VoltageSource  5 written + [wMechanical]
Machines.SMEE_Rectifier      7 written + [ir[2]]
FW.SMPM_Inverter             4 written + [smpmM.wMechanical, smpmE.wMechanical, smpmE.ir[2]]
FW.SMR_Inverter              4 written + [smrM.wMechanical, smrE.wMechanical, smrE.ir[2]]
FW.SMEE_Rectifier            7 written + []
QS.SMPM_Mains                3 written + [smpmQS.wMechanical, smpm.wMechanical, smpmQS.gamma, smpmQS.gammar]
```

So the family is one and its name is the rotor: the machines pin flux
states of the air gap that the demoted `wMechanical` and `ir[2]` do
not reach, while contributing conditions of their own. The `FW`
entries are the same machine twice over in one model - an `M` and an
`E` copy compared against each other - which is why their surplus is
double, and not because a quantity has two components. `FW.SMEE_Rectifier`
settles it outright: its demoted list is empty and it is still
lopsided by one, with eight pinned states, so there is a case in this
row where no demoted condition is involved at all.

The next question is therefore about the pinned side rather than the
condition side - why `airGap.psi_ms[1]` and its kin are still counted
as pinned unknowns when the written section and the machine's own
structure speak about them. `oxidelica why` on a single flux answers
it in one command, and the answer is that they are not five
independent unknowns:

```text
smpm.airGap.psi_mr[1] = L[1,1]*i_mr[1] + L[1,2]*i_mr[2]
smpm.airGap.psi_ms[1] = R[1,1]*psi_mr[1] + R[1,2]*psi_mr[2]
smpm.airGap.psi_ms[2] = R[2,1]*psi_mr[1] + R[2,2]*psi_mr[2]
```

`psi_ms` is the rotation of `psi_mr`, written out as an equation of
the air gap. So a machine carrying both as states carries the same two
degrees of freedom twice over, and the initialisation counts a pinned
start for each copy. That is the aggregate fault these notes already
name, seen once more: a value read two ways, and the reading cannot
see that one of the two is a definition of the other. Whether the cure
belongs in index reduction, which should not have kept both, or in the
count, which should not pin a state a definition reaches, is the
question the next shift takes - and the surplus of two in
`SMPM_Braking` is exactly the size of one such duplicated pair, which
is what the arity hypothesis was mistaking for two components.

The batteries that left the same row have their own fork named, and it
is not this one. `CCCV_Cell` now stops at `singular Jacobian in
algebraic loop ["cell.p.v", "cell.heatFlowSensor.port_a.Q_flow",
"cell.cell.currentSensor.p.v", "cell.cell.ocv.p.i"]`, and the probe on
`cell.p.v` shows a variable bound to nothing, standing in three
equations that all relate it to another potential -
`cell.v = cell.p.v - cell.n.v`, and two connections equating it to a
sensor's pin. A loop of pure potentials with a heat flow in it, which
is a different layer again from the initialisation count and is
recorded here as a fork rather than taken.

## The nominal attribute, and what it can and cannot buy

The 26 models refused with `the equations cannot be evaluated at the
values the block starts from` were counted by what their unknowns had.
Measured over `/tmp/p179-raw.txt` with `oxidelica why` on each
unknown, 71 of 99 non-derivative unknowns had no `start` at all, and
the rest fell into two families. The machines - `smpm.airGap.gamma`,
`i_sr`, `i_rr`, `spacePhasorS.i` - declare types with no `nominal`
whatever (`type Reluctance = Real(final quantity=..., final unit=...)`
and its five neighbours), so nothing can be read for them. The fluids
are the opposite: `Media` writes `nominal` on the type for nearly
every quantity it defines - `type SpecificEnthalpy =
SI.SpecificEnthalpy(nominal = 1e6)`, `AbsolutePressure(nominal =
1e5)`, `Density`, `Temperature` - and none of those declarations carry
a `start`. So the ceiling of a nominal-reading change was the nine
Fluid models of the row, and it was known before a line was written.

Read and seeded, the corpus moved by one: 868 flatten and 520 run
against 868 and 519, from one binary with the seed behind
`OXIDELICA_NO_NOMINAL_SEED` (`/tmp/n190-on.txt`,
`/tmp/n190-off.txt`). The diff of run lists names the one gained,
`ModelicaTest.Fluid.TestComponents.Valves.TestValvesCompressibleReverse`,
and no victims. The census moved by two rows and nothing else: `at t =
0.000000: Error in region computation of IF97 steam tables` fell from
2 to absent, and one `algebraic loop diverged` on
`simpleGenericOrifice.V_flow` appeared. That is the shape the notes
predict - a wall removed uncovers the next one - and it says the
remaining eight Fluid models of the row are held by something past the
first guess rather than by where they begin.

## One statement counted twice, at the edge of a coarse walk

The machines' `initialization is not square` was not a pair of
two-component quantities and not a defect of index reduction. The
reduction was measured on `SMPM_Braking` and it did see the rotation
`psi_ms = R*psi_mr`: its first reduction demoted `psi_ms[2]` on the
strength of that very equation. What refused the model was the count.

`OXIDELICA_INIT_PROBE=1` prints what each initial condition reaches
and what it claimed. Before the change, on `SMPM_Braking`:

```text
init: written smpm.is[1] = 0 reaches [], took nothing
init: written smpm.is[2] = 0 reaches [], took nothing
init: written smpm.i_0_s = 0 reaches ["smpm.lszero.i"], took smpm.lszero.i
init: demoted smpm.wMechanical reaches ["smpm.inertiaRotor.w"], took smpm.inertiaRotor.w
init: state smpm.airGap.psi_ms[1] claimed by nothing
init: state smpm.airGap.psi_mr[1] claimed by nothing
```

The reachability walk does not go through a simultaneous block, on
purpose - its coarseness would unpin states an equation does not
determine. So `is[1] = 0`, whose stator current the air gap's block
solves, reaches nothing and claims nothing. But it was still counted
as a condition, and the state it settles, claimed by nobody, was then
pinned at its declaration. One statement on both sides of the
arithmetic: four conditions for five unknowns, three of which were
pinned.

The two halves are the same loss seen from its two ends, and that is
what licenses giving it back: a written condition that claimed nothing
is paired with a state that was claimed by nothing. Neither half alone
would do - a condition naming a parameter the initialisation solves
for claims no state and means to, which is what a test caught and what
narrowed the rule.

Measured from one binary over `.msl`, the change behind
`OXIDELICA_NO_PAIR_LOST` (`/tmp/m193/before.txt`,
`/tmp/m193/after.txt`): 753 flatten and 488 run on both sides, and the
two run lists are identical name for name. What moved is the barrier:
the `not square` rows counted 19 models before and 2 after. Seventeen
models one storey up, none lost - the shape the notes predict for a
wall removed. The control the plan named, `FW.SMEE_Rectifier`, whose
list of demoted starts is empty, moved with them and now stops at a
singular Jacobian, so the fix explains it rather than leaving it to
another family.

The two that remain are not the same wall wearing the same words:
`CompareLineTrunks` has 60 conditions for 57 unknowns and no fixed
starts at all, which is a section that over-determines its own model
rather than a count that lost a claim.

## The air-gap family is one layer, and the layer is index reduction

Twenty-seven models of the register mention `airGap`, spread over
seventeen rows. Added by family the way the notes require, and with
`cannot differentiate` beside them for scale (numbers from
`grep -E "^\s+[0-9]+\s+.*airGap" /tmp/m193/after.txt | awk`):

```text
airGap ................ 27 models
cannot differentiate ... 16 models
```

The seventeen rows read as five different walls - a flux against an
inductance, a rotation matrix, a singular Jacobian, a current through
that matrix, a derivative of something that is not a state. Probed
with `library check .msl --only <Class>`, four of the five are one
wall: the same algebraic loop about `*.airGap.gamma`, and which
equation the register quotes is merely which one the block reached
first. The fifth, `IMC_Initialize`, is a different question and was
not probed further.

The mechanism, found by shrinking rather than by reading the register.
`AirGapS` writes its inductance as a matrix:

```modelica
parameter SI.Inductance L[2, 2] = {{Lm,0},{0,Lm}};
```

so `L[1,2]` is an outright zero, and `psi_ms[1] = L[1,1]*i_ms[1] +
L[1,2]*i_ms[2]` does not mention `i_ms[2]` at all. Index reduction
divides through by that coefficient anyway, and the definition it
builds carries `/ (-aimc.airGap.L[1,2])`. What the run then says is

```text
... is inf at t = 0, before any Newton step: the equations cannot be
evaluated at the values the block starts from
```

which names the solver, the one place nothing is wrong. `oxidelica why
IMC_DOL 'aimc.airGap.L[1,2]'` answers `bound to: 0`, so the compiler
knows the value; the layer that divides simply did not ask.

The layer is named precisely. `solve_linear_known` already refuses a
slope that is zero once the parameters are folded in, and the plan's
layer passes the table. Index reduction, in `compile.rs`, calls
`solve_linear_for` instead - the same solver with an empty table - in
two places: where it gathers candidate definitions, and where it asks
whether an equation can be rearranged at all. A probe printing the
table's size at the refusing call site prints `n=0` for every air-gap
slope.

Handing the parameters to both calls does move the wall. `IMC_DOL`
goes from `inf ... before any Newton step` to `singular Jacobian in
algebraic loop [...]` - one storey up, the shape the notes predict.
But it is not shippable as it stands, and the reason is the rule about
phases with no ceiling. Measured from one binary over `.msl` with the
change behind `OXIDELICA_NO_REDUCTION_PARAMS`, the run without it
finished its read half in 826s (`/tmp/m194/before.list`, line
`read 1040 of 1040 models, 826s so far`); the run with it stood at
`read 832 of 1040 models, 392s so far` (`/tmp/m194/after2.list`) and
did not move for over fifty minutes while its processor time kept
climbing. Timed one at a time, `IMC_DOL` costs 0.50s with the change
against 0.38s without, and `SMEE_DOL` 0.42s either way - so the cost
is not spread, it sits in whatever model the pass never got past.

Two things follow for whoever takes this next. The fix is right in
kind and wrong in price: folding the whole parameter table into every
slope of every reduction is the dear-test-first fault these notes
already name twice, and what it wants is the cheap question asked
first - is any name of this slope a parameter the table calls zero -
before anything is substituted. And the small model is a genuine gap:
thirteen shapes were tried, and while a probe shows the reduction
layer taking the two readings apart on a four-equation model, none of
them differs in outcome. The fault needs a chain deeper than a small
model carries, so the corpus is the only witness this one has.

## The phase without a ceiling, named

The model is `Modelica.Mechanics.MultiBody.Examples.Elementary.RollingWheel`,
with `RollingWheelSetDriving` and `RollingWheelSetPulling` beside it,
and it was named by a probe rather than by a tail. The corpus counter
says how many models have _finished_, so the three that never finish
leave no mark on it; printing `start` and `done` around each model and
subtracting the two lists names them in one pass. The tail of the log
names Fluid, as it did on shift 119 and shift 128, and it is a lie both
times - notes and counters reach the disk out of order.

The phase is index reduction, and it was named by sampling the wedged
process: 6745 of 7477 samples stood in
`reduce_index -> solve_linear_known -> substitute`. `RollingWheel`
with the change switched off takes 9s to flatten and 17s to run; with
it on the reduction did not come out after twenty minutes.

The cause is not the change. The change merely handed the layer slopes
big enough to show a fault already there: the slope was folded one
parameter at a time, and each `substitute` rebuilds the whole tree, so
the work is the size of the slope times the number of names in it.
A wheel rolling on a surface differentiates into an expression with
thousands of references, and the square of that is the phase with no
ceiling. Two changes together answer it - the cheap question first,
which is whether the slope names anything the table knows at all, and
one walk for the whole table where there were as many walks as names.

Measured from one binary over `.msl` with the fold behind
`OXIDELICA_NO_REDUCTION_PARAMS` (`/tmp/m195/before.list`,
`/tmp/m195/after.list`): both halves read 1040 models, both flatten 868
and run 520, and the diff of the two run lists is empty. The time per
model is 2193ms flattening and 1840ms running without the change,
against 2262ms and 1847ms with it.

What moved is the register, by name. Four rows quoting
`der(aimc.airGap.psi_ms[1])` and its `aimcE`, `aims`, `aimsE`
namesakes, together with a fifth quoting `aimc.airGap.i_ss[1]`, are
gone; `singular Jacobian in algebraic loop ["aimc.airGap.gamma", ...]`
arrives with 7 and its `aims` twin with 1. Eight models one storey up,
no model lost, and the wall they now stand at is an honest statement
about the model rather than an infinity charged to the solver.

### The accounting of that change, by name

The paragraph above said eight models moved a storey. The register says
eleven, and the difference is not a rounding: the two censuses were
compared line by line (`/tmp/m195/before.list`, `/tmp/m195/after.list`)
and every row that changed is named here, because a count of arrivals
that is smaller than the count of departures means some arrival was not
looked for.

Departures, ten machines: the four rows quoting
`der(aimc.airGap.psi_ms[1])` and its `aimcE`, `aims`, `aimsE`
namesakes carry 5 + 1 + 1 + 1 = 8 (before.list:1743, 1847, 1849, 1851),
and `aimc.airGap.i_ss[1] = ...` carries 2, first on
`Electrical.Machines.Examples.InductionMachines.IMC_Inverter`
(before.list:1773). Eleventh is `Mechanics.MultiBody.Examples.Loops.Fourbar2`,
which stood at `structurally singular model: equation Ref("j2.frame_a.R.T[3,...`
(before.list:2091).

Arrivals, the same eleven: `singular Jacobian in algebraic loop
["aimc.airGap.gamma", ...]` with 7 (after.list:1741), its `aims` twin
with 1 (after.list:2021), a row that did not exist before -
`singular Jacobian in algebraic loop ["idealCloser.idealClosingSwitch[1].s", ...]`
with 2, first on
`Magnetic.FundamentalWave.Examples.BasicMachines.InductionMachines.IMC_DOL`
(after.list:1809) - and Fourbar2's new wall (after.list:1885). Seven and
one and two and one is eleven. The `idealCloser` row is distinct from the
`idealCloserM` row of 1, which both censuses carry unchanged.

What the two new walls say, asked with `--only` from the root of `.msl`:

IMC_Inverter and IMC_DOL both land on a singular Jacobian over a loop of
thirty-odd names, headed in the first case by `aimc.airGap.gamma` and in
the second by the three switch states `idealCloser.idealClosingSwitch[i].s`.
The heads differ, the family does not: both are the machine loop, and the
switch names sort to the front in the model that has a closer in it.

Fourbar2's wall is the interesting one, and it is not the dishonest sort
the change removed from the machines. It reads

```text
`universalSpherical.f_b_a[1] = ...` of algebraic loop [...] is NaN at
t = 0, before any Newton step: the equations cannot be evaluated at
the values the block starts from; the block's own values are not
numbers: ["b2.v_0[2] = NaN", ...]
```

- a refusal that names the equation, the loop, the instant, and the
  hundred values that are already NaN when the block is entered. That is a
  statement about the model's start values rather than an infinity charged
  to the solver, so the model went from a structural refusal to a numerical
  one and the new one says more than the old. It is a candidate for the
  queue on its own terms: something upstream is producing NaN before the
  loop is ever solved.

## The row of ten that named nobody

`structurally singular model: cannot differentiate this expression` was
the top of the run half's register at 10 models, first on
`Modelica.Fluid.Examples.NonCircularPipes` (`/tmp/m195/after.list:1737`).
It was one row because the refusal said nothing: the catch-all at the
end of the differentiator returned a fixed sentence, so every model that
reached it sorted under the same words whatever it had actually met.

Asked the gate's question first - one family or several - with the
corpus register printed per model (`/tmp/m196/refused.txt`, `library
check .msl --refused --without scripts/heavy_models.txt`). The subject
is wider than the row: summed over every line mentioning "cannot
differentiate", the register carries 10 + 4 + 1 + 1 = 16, the other
three rows being the two that name a function by name and the one that
names a non-constant exponent.

The ten are not one family, and the register with the refusals named
says how they split. Measured from the two censuses over `.msl` with
`--without scripts/heavy_models.txt`, both reading 1040 models, both
flattening 868 and running 520 (`/tmp/m196/refused.txt`,
`/tmp/m196/after_refused.txt`):

```text
before                                    after
10  cannot differentiate this expression   6  cannot differentiate a subscript that survived flattening
                                           4  cannot differentiate a call of several arguments
 4  cannot differentiate function `abs`    4  cannot differentiate function `abs`
```

The same sixteen models, model for model, and no count outside the row
moved. The six are one shape, and it is a shape these notes already have
a name for: a subscript over a call into a water medium -
`annulus_pipe.mediums[1].d`, `reservoir.medium.h`, `state.T`,
`volume1_2.medium.d`, `mixingVolume2.medium.d`, `junctionVolume.medium.d`,
each `...IF97_Utilities.waterBaseProp_ph(...)[5]`. That is not a missing
derivative rule; the arrays were meant to have come apart long before the
differentiator saw them.

The other four are rules that could be written, and they are two calls
and not one: `.atan2` in
`MultiBody.Examples.Constraints.PrismaticConstraint`, and `min(...)` in
`ModelicaTest.Media.TestOnly.DryAirNasa` and the two
`Tables.CombiTable2D{s,v}.Test33`. Reading the equations by eye before
the refusal was fixed had put the last three under a different shape
entirely - the quoted equation was `Ref = Ref` and the call was buried
inside it. The refusal that names the expression it refused got it right
where reading the equation did not, which is the whole of the case for
naming it.

So the refusal now names two things: which construction was met, which
is the family, and the expression as it was written, which is what finds
the model. It says `cannot differentiate a call of several arguments`
and quotes the call, where it used to say `cannot differentiate this
expression` and quote nothing. The kinds are matched by name and not swept up, so a
variant added to `Expr` has to be decided about rather than quietly
joining the refusal - the same rule the run's `shape_of` already keeps.

## What a ceiling over flattening would cost, measured but not built

The run half has two working ceilings - `MAX_EVENTS_ONE_INTERVAL` and
`MAX_ROWS` - and both fire while the run is going on. Flattening has
`MAX_DEPTH` and `MAX_WHILE_ROUNDS`, which guard recursion and a loop
rather than volume, and `FLATTEN_MS_CEILING`, which is checked against
the totals of a finished pass (`scripts/library_floor.sh:179`) and so
is behind a door that a wedged model never opens. `RollingWheel` cost
half a shift and a burned corpus through that gap.

There is already a ceiling of the right shape in the compiler, and it
is worth naming because it makes the work small rather than novel:
`max_constraint_nodes` counts the nodes of a differentiated equation,
refuses past a fixed number, and names the equation and the reduction
it stood at (`crates/oxidelica-sim/src/compile.rs:662, 1398`). It even
carries the thread-local lowering a test needs, since the environment
belongs to the binary and the tests share one.

What it should count is nodes, not time and not substitutions. Time is
not reproducible across the two machines these notes already keep apart,
and a count of substitutions says nothing about the size of what each
one carried - which is exactly the fault that made the slope phase
unbounded. Nodes are what the compiler already holds and what grew: the
growth probe beside that ceiling prints an expression going 14k, 90k,
5M, 111M characters.

Where it belongs: the flat model's equation list as instantiation adds
to it, so the count is over what has been built rather than over one
expression. The cost of getting there is that the accumulator threads
through the whole of `flatten`, which is why this is a measurement and
not a change.

For scale: `MultiBody.Examples.Elementary.RollingWheel` now flattens in
8.6s and runs in 16.3s, and `Pendulum` beside it flattens in 4.2s
(`library check .msl --only`, one model each). Neither is near a wall;
the wall is what a model that never finishes needs, and nothing measures
it today.

## The eight singular initialisations, and which site raised them

The run census's top row was `the initialization problem is singular:
its equations do not pin the states down`, eight models, and the
sentence named no state. Two places raised it and they are different
positions - one where Newton has satisfied the equations and the
Jacobian is still singular, one where no step solves at all - so the
first question was which. All eight came from the second
(`compile.rs:4759`); the first fired for no model of the library.

The names the refusal now carries, one probe each, by group:

```text
AmplifierWithOpAmpDetailed               opAmp.v_in
DemonstrateLightning                     lightning1.signalSource.T10
Lines.LightningLosslessTransmissionLine  lightningImpulseCurrent.signalSource.eta
Lines.LightningSegmentedTransmissionLine lightningImpulseCurrent.signalSource.eta
Fluid.Examples.Tanks.EmptyTanks          tank1.U
ModelicaTest...Vessels.TestSimpleTank    tank.U
Media.Examples.ReferenceAir.DryAir1      volume.U
Media.Examples.ReferenceAir.DryAir2      volume.U
```

So the row is not one family but four: an operational amplifier's
input voltage, the impulse source's shape parameters, and two
independent arrivals at a vessel's internal energy. The three twin
pairs agree with each other, which is the check that the name is a
fact about the model rather than an artefact of the instrument - and
that check was worth making, because the cheap answer would have been
an artefact.

The cheap answer is the column Gaussian elimination stops on.
`solve_linear` pivots by row within a column and walks the columns in
the order the unknowns sit in the vector, so a degeneracy spanning
three states is blamed on whichever of them was declared first, and
swapping two declarations moves the blame while the model stays the
same. What is reported instead is the Jacobian's null direction, which
is the family of starting points the equations do not choose between;
every unknown with a component in it is unpinned. Measured on a
twelve-line model, the set is the same with the declarations in either
order and only the listing order differs.

Three of the four groups point at a quantity a component computes
rather than at a state the model declared - `U` is a vessel's internal
energy, `eta` a shape parameter of the impulse. Where those come from
is the work behind this row; the refusal now says which name to ask
`why` about, which it did not before.

## How big a flat model gets, measured

`OXIDELICA_SIZE_PROBE=1` prints the size of every flat model as
flattening finishes it: nodes in the equations, nodes in the bindings,
the counts of each, and the largest single equation. It prints and
does not refuse - the ceiling the previous section argued for needs a
number from an instrument rather than from a head, and this is that
instrument. The equation list only grows during flattening, so its
final size is its peak, and counting once at the end says what an
accumulator threaded through twenty-one push sites would.

Over the corpus, the 868 models that flatten, heavy ones left out
(`/tmp/m197/size2.txt`):

```text
p50    805 nodes
p90 13,074
p99 1,132,197
max 2,196,724   MultiBody.Examples.Loops.Fourbar2
```

The distribution is not a slope, it is two populations. Forty-two
models sit above a million nodes and the rest below a hundred
thousand, and there is nothing in between worth the name. That gap is
the finding, and what is in it is sharper still: all forty-two of the
models above a million, and no others, carry one and the same
equation, of 80,652 nodes, for `world.x_label.R_rel[2,1]` - the
orientation of the label drawn on an axis of the world's coordinate
frame. Every MultiBody model instantiates `world`, and `world` draws
its axes.

So the count says a MultiBody model is a million nodes, and the reason
is not the mechanism it models. `Pendulum` carries 1,093,532 nodes
over 934 equations, and `ModelicaTest.Rotational.TestMove` - no
multibody mechanism in it at all - carries 1,093,585 over 954, the two
within a twentieth of a percent of each other because both are almost
entirely the same world's labels. The physics of a four-bar linkage is
the difference between them and the maximum.

Two things follow. A ceiling chosen off the raw maximum would be set
by an annotation rather than by a model, and 42 models would sit just
under it for a reason none of their authors would recognise. And the
thing worth looking at before the ceiling is the label equations
themselves: a shape that is the same in every model of a library, that
nothing in the run needs, and that costs more than everything else
those models contain put together.

## Why eight initialisations come out singular

The eight models behind `the initialization problem is singular: the
Newton step does not solve` were named last shift. This is what stands
behind the names, probed one family at a time with the Newton's
starting point and Jacobian printed out.

They are not one cause. They are two, and the split is exactly where
the sizes of the unknowns are.

### The instrument was lying: a step that is not small

`DemonstrateLightning` leaves `signalSource.T10` free. The first
suspicion - that a `fixed = false` parameter never received the
`start` MSL gives it, or lost it passing through an `if` - is wrong,
and cheaply so. A ladder of three two-line models, `start` a literal,
then an expression over another parameter, then an `if` expression,
all run; and made to pick between roots by a negative start, all three
pick the root the start is nearest. The starts arrive. Printed at the
model itself, they arrive there too: `T10 = 3.659917e-7`,
`tau1 = 3.5e-4`, `tau2 = 1e-5`, every one the value its declaration
asks for, in both branches of the `if` the flattener had to choose
between.

What was wrong was the derivative, not the point it was taken at. The
Jacobian's finite-difference step was `1e-7 * (1 + |y|)`, which is a
formula for quantities of order one and floors at `1e-7` outright.
`T10` starts at `3.7e-7`: the step is a twenty-seven percent shift of
the unknown, and under the fifth power of the Heidler peak condition
what comes back is a chord across half an arc rather than a tangent.

The measurement that settles it is not whether a model runs but the
Jacobian itself, on a probe whose columns have an exact ratio. Two
unknowns, `(a/b)^5 = 32` beside `a + b = 3s`, swept over the scale
`s`. At `s = 1`, the ratio of the columns is `-0.818`, which is the
truth. At `s = 1e-7`, the same model, the same equations, the ratio
is `-25`. The instrument stops being an instrument somewhere around a
microsecond, which is precisely where the electrical library's time
constants live.

Made relative to the unknown, the instrument tells the truth at every
scale: the same probe gives `-0.818` at `1e-7` as it does at one.
Swept over the Heidler peak condition, the old formula stops
converging below `1e-8` while the relative one returns 2.359701, the
number the model gives at every scale from `1e-3` down.

And the change was reverted, because the corpus priced it: 520 models
ran before and 517 after, with nothing gained. The three that left
are named - `SMPM_VoltageSource`, `IMC_YD`, `SMEE_Generator`, all
magnetic machines - and what they say about the step is the finding
rather than the loss. Every one of their initialisation unknowns
starts at exactly zero, where the two formulas give the same step;
they were running on the absolute floor carrying them away from zero
in the first place. A step relative to an unknown that is zero is
zero, so the Jacobian goes blank, and blank is refused the same way a
lying one is.

So the two halves of the library want opposite things from one
constant. The electrical library's time constants need a step below
`1e-7` or the derivative is a chord; the magnetic machines need a
step above their own zero or there is no derivative at all. A middle
was tried and measured - the step scaled by the larger of the value
and its `start` - and it recovers nothing, because those starts are
zero too. Flooring the scale at one recovers all three machines and
is, at small scale, exactly the old formula again, so it gives the
lightning back.

What that says is that the step does not want a better constant, it
wants a scale per unknown, which is what `nominal` is for and what
this Newton does not read. That is the shape of the fix, and it is
larger than a constant.

### The other three are a state the plan also computes

The prediction that came with the suspicion was that this cause must
not explain the tanks and the air, whose free names are internal
energies and therefore large. It does not, and the probe says why in
one line: their Jacobian columns are not inaccurate, they are exactly
zero.

`EmptyTanks` leaves `tank1.U` free, `DryAir1` leaves `volume.U`,
and both carry the same pair of equations:

```text
volume.U = volume.m * volume.medium.u
der(volume.U) = volume.port.H_flow
```

`U` is a state, and `U = m*u` is an equation the algebraic plan
computes something from. Which something was guessed wrong here and
is worth correcting where it was written: the guess was that the plan
recomputes `U` and writes over the slot the state sits in, so that a
perturbation is erased before any residual sees it. A probe printing
every state whose slot holds something other than what was placed in
it fired not once on `EmptyTanks`. The plan never touches a state's
slot, and it cannot: a state is not among the unknowns the plan is
built over.

What the plan does with `U = m*u` is solve it for `u`, which is the
one unknown in it, and the assignment it writes is `u := U/m`. `m` is
the other state, an empty tank starts it at the zero its declaration
left it, and so the very first evaluation of the initialisation
divides by zero. Everything below comes out `NaN`, the difference
quotient of a `NaN` against a `NaN` is a `NaN`, and the column is
recorded as exactly zero. The Jacobian probe says both halves in one
breath, which is what settled it:

```text
jac: column tank1.U (y = 0) has magnitude 0
jac: row written #0 residual NaN magnitude 0
```

The zero of the column is the consequence, and the `NaN` of the
residual is the cause. A column that reads zero has three possible
causes - a slot overwritten, a slot never read, a name that cancels -
and none of them is this one. The order matters: had the residual
been read first, the slot theory would never have been written down.

The amplifier is the same shape wearing electrical clothes:
`opAmp.v_in` is a state, because `i_c3 = Cin*der(v_in)` differentiates
it, and it is also written outright by `v_in = Rdm*i_r2`. Three of
the four families are this one cause, and it is a fluid one only by
accident of which libraries write energy balances.

This is not the step, and it is not the model. It is a state whose
slot the plan owns, and it wants its own fix.

### The families, and where each is

| model                        | free name          | cause                                                                   |
| ---------------------------- | ------------------ | ----------------------------------------------------------------------- |
| `DemonstrateLightning`       | `signalSource.T10` | step not small next to a microsecond; three shapes measured, all parked |
| `EmptyTanks`                 | `tank1.U`          | `u := U/m` divides by an empty tank's zero mass; cured, the model runs  |
| `ReferenceAir.DryAir1`       | `volume.U`         | not this cause: refuses identically with the cure switched off          |
| `AmplifierWithOpAmpDetailed` | `opAmp.v_in`       | not this cause: refuses identically with the cure switched off          |

### The cure, and what it cost

The same fault as a torn block that starts on a reciprocal's pole,
which the run already knows how to treat: it retries such a block
from off the zero rather than giving up on it, because the point the
iteration was handed is the only thing wrong with it. The
initialisation now does the same. If its residual at the guess is not
made of numbers, and some unknown stands at exactly zero, the zeros
are moved to `1e-6`, `1e-3`, `1`, `1e3` in turn and the first point
whose residual is a number is where Newton starts. An initialisation
whose residual is already a number never reaches this.

Which magnitude serves is the model's own scale and this layer does
not know it, which is the same argument the block retry makes and the
reason several are tried rather than one chosen.

Measured as a pair from one binary, the change behind
`OXIDELICA_NO_INIT_ZERO_STEP`: 520 models run against 526, and the
six are named, with nothing leaving and the flatten list identical.

```text
Modelica.Fluid.Examples.Tanks.EmptyTanks
Modelica.Fluid.Examples.Tanks.ThreeTanks
Modelica.Mechanics.MultiBody.Examples.Elementary.PointGravity
Modelica.Mechanics.MultiBody.Examples.Elementary.SpringWithMass
ModelicaTest.Fluid.TestComponents.Valves.TestDelayedValve
ModelicaTest.Fluid.TestComponents.Vessels.TestSimpleTank
```

Only one of the four families named above is among them, and that is
worth saying plainly because the table promised three. `EmptyTanks`
runs. `DryAir1` and `AmplifierWithOpAmpDetailed` refuse with the same
sentence about `volume.U` and `opAmp.v_in` with the switch either
way - the cure does not reach them, so they were never one family
with the tank, and the guess that made them one was the slot theory
this section began by correcting. The five that came in beside the
tank were not predicted by the table at all: `ThreeTanks` is its
sibling, but `PointGravity` and `SpringWithMass` are mechanical and
had nothing to do with any energy balance. A cause named by its
mechanism reaches models no census of symptoms would have grouped
with it, and misses models a census of symptoms did group.

## The Newton difference step: the third variant, measured and parked

Two shapes of the step had been measured before: relative to the
unknown, which kills the machines because every one of their unknowns
starts at a true zero, and a floor of one, which is the old formula
wherever the scale is small and so does nothing for the lightning. A
third was left unmeasured and looked like it answered both: a step of
the unknown's own scale, `1e-7 * max(|y|, |start|)`, falling back to
an absolute `1e-7` only where that maximum is exactly zero. On paper
the lightning gets a step of `3.7e-14` against its `3.7e-7`, and the
machines, whose maximum is zero, get the old absolute step to the
digit.

Measured behind `OXIDELICA_SCALED_STEP` in both places a difference
is taken - the run's Jacobian and the initialisation's - it fails on
both sides:

```text
DemonstrateLightning   off: refuses on signalSource.T10
                       on:  refuses on signalSource.T
machines (3 named)     off: 3 flatten, 3 run
                       on:  3 flatten, 0 run
```

The lightning does not arrive. What changes is only which name the
refusal prints, `T10` becoming `T`, which is the signature of a
different column going free rather than of a model that got closer to
running. And the machines leave, all three, despite the argument that
for them the formula is unchanged - so the argument was wrong
somewhere, and the place it is wrong is `|start|`: a machine's
unknowns are not all at a true zero after all, and those with a start
get a step scaled to it, which is not the old one. Reverted.

### And the measurement that was supposed to come first

The maliava asked for a cheaper check before any of this: run the
three machines at two absolute step sizes and compare the final
numbers rather than the fact of running, because holding a model in
the floor on a derivative one has called a lie is not a win. Behind
`OXIDELICA_JACOBIAN_STEP`, at `1e-7` and `1e-6`, comparing the last
row column by column and ignoring columns below `1e-6` in magnitude:

```text
SMPM_VoltageSource   worst relative difference 2.3e-4 (der(smpm.airGap.V_msr.re))
SMEE_Generator       worst relative difference 0 exactly
IMC_YD               does not finish at 1e-7 at all
```

Two of the three are solid: their numbers do not depend on the step,
so what they run is an answer and not noise, and the maliava's
criterion of keeping them stands for those two. The third is neither
solid nor noise - it converges at one step and not at the other,
which is the edge rather than a wrong number, and it sits in the
floor on a coin toss. Recording it as "the machines do not leave" was
the thing to avoid.

Two cautions about that measurement, both of which narrow it. The
switch sat in the run's Jacobian rather than in the initialisation's,
so what was compared is wider than the initialisation question that
prompted it, and IMC_YD's failure at `1e-7` is a runtime algebraic
loop rather than an initialisation. And a flat `1e-7` is smaller than
anything production takes, since `1e-7 * (1 + |y|)` is at least
`1e-7` and twice that for an unknown of order one - so "does not run
at 1e-7" is a statement about a step the compiler never chooses.
Asked of the shipping binary with no switch at all, IMC_YD runs.

## The row that was two thirds a dead column

The tank family came off the top of the map, so the census was taken
fresh (`/tmp/m200/census.txt`, 868 flatten and 526 run). The top of
the run half is algebraic loops with about a hundred models under
them: 34 `singular Jacobian`, 30 `residual ... of algebraic loop`,
10 diverged, 10 that did not converge in fifty Newton iterations, 8
underdetermined. The top single row, 34, was the one probed.

A kind is not a family, so the probe was aimed at the three smallest
members - blocks of one unknown, where the Jacobian is a single
number and there is nothing to read wrong. All three printed the same
thing: a finite non-zero residual and a Jacobian of exactly zero.

```text
GearType2          v=[0.0]  f=[21.8]     jac=[[0.0]]
TestSuddenExpansion v=[0.0] f=[-10000.0] jac=[[0.0]]
Oscillator         v=[0.0]  f=[-1e-7]    jac=[[0.0]]
```

Scanned over sixteen decades of step, the Oscillator's residual does
not move by one part in anything: the difference is `0e0` from `1e-8`
to `1e6`. That is not a matrix that came out ill conditioned, it is
the block declining to mention its unknown at all.

Two different sources, one column. `T1.irc * T1.p1.m_collectorResist
= T1.C.v - T1.Cinternal` has a collector resistance the Spice3 model
card declares as `RC = 0.0` and no example overrides, so the
coefficient is a parameter that is literally zero.
`bearingFriction.sa` is dropped by the branch of the friction `if`
that holds while the bearing is locked - the same shape as the
`semiLinear` coefficient already recorded above, one storey further
along, where the branch is chosen by a discrete mode rather than by a
sign. A parameter that is zero and a branch that does not mention it
arrive at the same place: the matching pairs the unknown with an
equation that says nothing about it, and Newton is handed a column
of zeros.

**What the change was: a refusal renamed, and nothing else.** Where a
column is exactly zero the refusal now names the unknown and says the
equations do not mention it; where no column is zero it says
`singular Jacobian` exactly as before. Measured with the register
before and after (`/tmp/m200/census.txt`, `/tmp/m200/census2.txt`),
the row of 34 came apart into 22 and 12 - two thirds of it was a dead
column wearing the words of an ill conditioned matrix - and the
counts did not move: 868/526, 753/494 on both sides. This is a wall
named rather than a wall removed, and the two are separate claims.

Which makes the 22 the family to work, and it is a family rather than
a row: the probe put every one of them in the same layer. What stands
behind them is the pairing, and the question is whether `solve_shape`
should refuse a name whose coefficient is zero _by branch_ the way it
already refuses one whose slope folds to a literal zero.

### The external functions, counted honestly before being left

The top of the flatten half is 11 models refused for a function
written in C that this compiler has none of its own for. Twenty-four
such functions are already carried, the whole `CombiTable1D/2D/
TimeTable` family among them, and the 11 that remain want only five:
`impureRandom` (a documented xorshift, written out in the MSL
itself), `writeRealMatrix` (MAT5 is already read here, with code and
tests - what is missing is writing), and `countLines`,
`getNumberOfFiles`, `getEnvironmentVariable`, a few lines each.

The honest count is 3 + 8, not 11. Only three are examples of the
standard library - `Blocks.Examples.Noise.ImpureGenerator`,
`Noise.Utilities.ImpureRandom`,
`Utilities.Examples.WriteRealMatrixToFile` - and the floors count
those. The other eight are `ModelicaTest`: `Tables.Test25_usertab`,
`Test18_usertab` twice, `Utilities.TestInternal`, `TestReadFile`,
`TestStreams`, `Math.TestColorMapToSvg`,
`Math.Random.TestRandomIntegers`. So the ceiling of that work is
three models, against about a hundred standing behind the loops, and
it is left where it is with its five names written down.

## The thirty that are not numbers before the first Newton step

The biggest single row of the run half is 30 models refused for a
residual that is NaN or -inf at `t = 0`, before any Newton step. A row
is a kind and not a family, so the thirty were split by the mechanism
the refusal quotes - mutually exclusive, by first match, summing to
exactly 30 (`grep "before any Newton step" /tmp/m200/raw2.txt`):

```text
IF97 (waterBaseProp_ph) ........... 7
machines (airGap.RotationMatrix) .. 7
MultiBody ......................... 6
orifices .......................... 4
FluxTubes (R_m = 1 / G_m) ......... 4
transistors ....................... 2
```

The IF97 seven are `Fluid.Examples.DrumBoiler.DrumBoiler` and six of
`ModelicaTest.Fluid`: `TestDensity`, `TestTemperature1`,
`DynamicPipesAndFittings`, `BranchingPipes1`, `BranchingPipes12`,
`BranchingPipes2`. The MultiBody six are `DoublePendulum`,
`ThreeSprings`, `RollingWheel`, `RollingWheelSetDriving`,
`RollingWheelSetPulling` and `Fourbar2`, the last of which is parked.

### Where the IF97 non-number was born

Probed on the smallest member, `TestDensity`. The refusal names
`simpleGenericOrifice.port_a_T` and `.d`, and both are downstream: what
they read is `waterBaseProp_ph(p, h, ...)` where `h` is a junction's
stream mix. Every port of that junction starts with a zero flow, so
the mix the flattener writes,

```text
(max(-m1, 0)*h1 + max(-m2, 0)*h2) / (max(-m1, 1e-10) + max(-m2, 1e-10))
```

evaluates to `0 / 3e-10`, which is exactly zero. Zero specific
enthalpy is outside the IF97 tables at any pressure - `region_ph(1e5,
0)` returns -1, the "outside of valid range" answer - and the medium
hands back NaN. The non-number is not born in the solver, nor in the
medium: it is born in the connection, where a floor was put on the
divisor and not on the weights.

The specification (15.2) uses `positiveMax` on both halves of that
fraction, and the floor there is not decoration. With it on the
divisor alone a node whose flows have all gone quiet mixes to zero,
which is not any port's value but the residue of dividing nothing by
the floor. With it on both, the quiet node mixes to the plain average
of what its neighbours hold, and a port that actually pushes still
drowns the floor by ten orders of magnitude - the weighted answers are
unchanged to a part in ten billion.

The small model that shows it whole: three ports on a node holding 100
and 200 with every flow zero mixed to 0 before the change and to 150
after.

### What the change was worth, both halves

Measured on one corpus pass (/tmp/m201/corpus.txt): 868 flatten and 528
run, runnable 753 and 496, against 868/526 and 753/494 before. Two
models won, both halves agreeing, and the flatten count untouched -
which it should be, the mix being read only at the run.

The two are named from the run lists rather than got by subtracting
the counts, which is the only way that distinguishes two arrivals from
three arrivals and a departure: diffed against the 526-line baseline
of the previous pass, `TestWaterPumpRecirculation` and `TestPressure`
came and nothing left. Both are `ModelicaTest.Fluid`, which is where
the mechanism said they would be, and `TestPressure` is one of the
four orifices - the sub-family the row's breakdown turned up and no
earlier reading had named.

The two are a small part of the story, and the rest is the more useful
half. Of the eleven Fluid models in that row - the IF97 seven and the
orifice four - only `TestPressure` runs outright. The others travelled
one storey: `TestDensity`, probed again, no longer refuses for a
non-number at all but for `algebraic loop did not converge in 50
Newton iterations`. The mix now hands the medium an enthalpy that is
inside the tables, the residual is a number, and what stands behind it
is the loop's own convergence - a different wall, and the next one to
work. This is a wall passed rather than a family finished, and the two
counts say so from either side.

## The wall behind `did not converge`, and what was under it

The prior shift's claim about `TestDensity` is confirmed, with a file
beside it. `library check .msl --only
ModelicaTest.Fluid.TestComponents.Sensors.TestDensity`, printed into
`/tmp/m202/testdensity_fable.txt`:

```text
of the 1 that flatten, 0 run:
      1  algebraic loop did not converge in 50 Newton iterations: ["s…
         first on ModelicaTest.Fluid.TestComponents.Sensors.TestDensity
```

Both the wording and the storey agree with what was claimed. That
model costs 13.6 seconds of one corpus pass on its own, fifty Newton
steps over a loop of orifice pressures - a sub-family that is dear in
minutes as well as in models.

### The family, laid out by what its loops are made of

Ten models, from `/tmp/m202/raw.txt`, split on the first match and
summing to ten:

| mechanism        | count | what the loop holds                                          |
| ---------------- | ----- | ------------------------------------------------------------ |
| ideal switches   | 6     | `idealDiode[i].s`, `idealThyristor[i].s`: a rectifier bridge |
| machine air gaps | 3     | `airGap.V_mss.re/.im` beside a converter current             |
| fluid orifices   | 1     | `TestDensity`: `simpleGenericOrifice.V_flow` and a density   |

The IF97 models that travelled a storey the previous shift did not
land here as a sub-family of their own: only `TestDensity` did, and it
is the one fluid member. The rest are elsewhere in the register.

### What the smallest member was actually doing

`Modelica.Electrical.Analog.Examples.OvervoltageProtection` is one
unknown, `zDiode.v`, and it refuses in a millisecond - the probe the
whole family is worth reasoning from. A trail printed at every Newton
step (`/tmp/m202/trail.txt`) says the loop was not diverging and was
not singular. It had _converged_:

```text
newton 13 t=0.0003 |f|=1.31e2   v=[-5.6453809968429995]
newton 14 t=0.0003 |f|=3.33e-5  v=[-5.645380947133184]
newton 15 t=0.0003 |f|=2.682209014892578e-7 v=[-5.645380947133196]
newton 16 t=0.0003 |f|=2.682209014892578e-7 v=[-5.645380947133196]
...
newton 49 t=0.0003 |f|=2.682209014892578e-7 v=[-5.645380947133196]
```

Thirty-five iterations reproducing one residual to the last digit,
then a refusal. The Zener diode's equation puts an exponential in
millivolts against a current through a parallel resistance of 1e8,
so both sides of the equation sit near a thousand million. Two such
sides agreeing to every digit double precision holds differ by around
1e-7, because that is where the rounding of 1e9 lands. The
convergence test asked for `1e-10 * (1 + |v|)`, and `v` is a volt: it
demanded 1e-10 of an equation whose arithmetic cannot resolve below
3e-7. Newton then stepped by nothing and got its own residual back,
which is exactly the trail above.

So the residual was judged against the _unknown_ and never against the
_equation_, and a solved block was called a failure. The rule taken:
an equation is also solved when its difference falls below the
rounding noise of the two sides it was subtracted from, `1e-12 *
(|lhs| + |rhs|)`. That is not a looser tolerance in the units of the
unknown - it is a floor no iteration can go under, whatever it does.
The two halves of each residual are kept for it, because a difference
alone no longer remembers the numbers it came from.

The small model is a single equation of that shape and refused before
the change: `i = 0.7*exp(-(v + 5.1)/(0.74*0.04))` against `i = (v +
5.7)*1e9`.

### What it was worth: a wall named, not a wall removed

One corpus pass, `/tmp/m202/corpus.txt`: 868 flatten and 528 run,
runnable 753 and 496 - the same five numbers as before, and the run
list diffed against the 528-line baseline in
`/tmp/m202/ran_before.txt` is identical line for line. No model won,
none lost.

What moved is the storey. `OvervoltageProtection` left the converge
row and appears in the `do not mention` row instead
(`/tmp/m202/overvoltage_after.txt`):

```text
the equations of algebraic loop ["zDiode.v"] do not mention
["zDiode.v"] at t = 0.0008: nothing in the block changes when it does
```

Which is the second wall in that model's way, and a real one: at
0.0008 seconds the diode is on the flat of its linear continuation
and the Jacobian's only column goes to zero there. The converge
family stands at ten either way - one member left and `TestDensity`
arrived - so the row's count is the same and its membership is not.
This is a wall passed and the family not finished, and the two counts
say so from both sides.

## The family of diverged blocks, and what it turned out to be

Sixteen models refuse with `algebraic loop diverged`, and they are not
sixteen problems. Read off the raw material of one corpus pass
(`/tmp/m202/raw.txt`, a model per `built` line), and taken by the first
match so the parts do not overlap:

| how many | what they are                            | the loop they share                                                    |
| -------- | ---------------------------------------- | ---------------------------------------------------------------------- |
| 5        | `TestWaterPump*` of `ModelicaTest.Fluid` | `pump.medium.p`, `pump.rho`, `pump.port_a.m_flow`, a valve's density   |
| 7        | `BranchingPipes*` and `SeriesPipes*`     | `mediums[1].p` of each pipe, with `dp_turbulent` of the valves between |
| 2        | `TestFlowRate`, `TestTemperature1`       | an orifice's `V_flow` beside the sensor's own medium                   |
| 1        | `DrumBoiler`                             | an evaporator's enthalpy against its level                             |
| 1        | `DynamicPipesAndFittings`                | a hundred and three unknowns over twelve pipes                         |

The loop sizes say more about the parts than the names do. The pumps
are 4, 5, 5, 5, 6 - one size, which is an argument for one mechanism.
The pipes run 3, 6, 6, 6, 17, 19, 19 - sixfold at the same wall, so a
part named by its file name may be two by its substance. The last is
an order out on its own and is in the table alone for that reason.

One correction to a reading that looks plausible and is not:
`dp_turbulent` is not a parameter the compiler failed to evaluate. It
is declared a variable in `Modelica/Fluid/Valves.mo:21` - `dp_turbulent
= if not use_Re then dp_small else ...` - so standing unknown in a loop
is its right.

### What they diverge by: a step over the edge of a domain

`SeriesPipes2` is the smallest of the sixteen, three unknowns, and its
Newton trail (`OXIDELICA_NEWTON_TRAIL=1`, the run in
`/tmp/m203/sp2_trail.txt`) is two lines long:

```text
newton 0 |f|=9.99e4  v=[497500, 100000, 492500]        f=[-997, 99990, -997]
newton 1 |f|=NaN     v=[20848050, 10.0001, 20843867]   f=[NaN, 0.0001, NaN]
```

Nothing diverged. The first residual is finite and modest, the
direction is right, and the step is simply too long: it takes a
pressure from five atmospheres to two hundred, where the IF97
formulation of water has nothing to say and answers NaN. The solver
then reads a value that is not a number and calls the block diverged,
which names the iteration for a fault of the medium's domain.

The small model is that in twelve characters of arithmetic:
`1/sqrt(x - 3) = 10 + time`, started at `x = 4`. One Newton step from
four lands at minus fourteen, where the square root is not a number,
and the block was refused before this change with a root sitting at
3.01 a little way off.

Backtracking already existed in the solver, and was reached only after
the iteration had been seen to walk in a circle - which cannot happen
here, because the second point is not a point at all. So the retreat is
now hung on the residual rather than on the history: a step whose
residual is not a number is taken again at half the length, from the
footing it left, and the block keeps the shorter step for the rest of
the solve. Divergence is what is left when even a millionth of the step
cannot be evaluated.

### The bridges are the other thing entirely

`Modelica.Electrical.Analog.Examples.Rectifier`, the smallest of the
six bridges that say `did not converge in 50 Newton iterations`, does
not fail to converge. Its trail (`/tmp/m203/rect_trail.txt`):

```text
newton 1 |f|=4.2e-8   v=[-2.000000012, ... , 1.2e-8]
newton 2 |f|=9.31e-10 v=[-2.0, -2.0, -2.0, -2.0, -2.0, -2.0, 3.37e-14]
newton 3..49 |f|=9.31e-10 - the same to every digit
```

Six diode coordinates land on exactly minus two and stay; the seventh
residual sits at 9.3e-10 and does not move for forty-seven iterations.
That is the floor of the arithmetic again, and the floor test added
last shift does not catch it: that test compares the difference against
the two sides of the equation, and here the cancellation happens
_inside_ one side, between terms of a sum, so both sides are small and
their scale says nothing. The residual is noise at 1e-9 and the test
demands 1e-10 of it.

Which parks the six bridges with a name rather than a shrug: they are
not a switching cycle and not a singular column, they are a convergence
test that measures a sum's terms by the sum. Judging a residual against
the largest term that went into it is the shape of the answer, and it
wants the terms carried out of the residual the way the two sides
already are - a change to what the evaluator hands back, which is why
it is parked and not taken here.

### What the retreat was worth: ten models a floor up, none won

One corpus pass after the change (`/tmp/m203/corpus.txt`): 868 flatten
and 528 run, runnable 753 and 496 - the same five numbers, and the run
list diffed against the 528-line baseline in `/tmp/m203/ran_before.txt`
is identical line for line. Nothing won, nothing lost.

What moved is the family. The same sixteen models re-run on their own
(`/tmp/m203/div_after.txt`) now say:

```text
 4  singular Jacobian in algebraic loop ["pipe1.mediums[1].p", ...
 2  algebraic loop did not converge in 50 Newton iterations
 6  algebraic loop diverged (in four rows, by their loops)
 2  IF97 asked outside its region, at t = 0
 1  singular Jacobian ["evaporator.h_S", ...]
 1  a kind of its own
```

Sixteen became six. Ten models walked past the edge of the domain and
died at the next wall along - four at a singular Jacobian, two at the
iteration budget, two at the medium refusing outright with its own
message, which is a better refusal than a NaN read back as divergence.
The six that remain go over the edge again further in, where the
shortened step cannot bring them back: `SeriesPipes2` retreats
successfully at its second iteration, walks eleven more, and meets a
second edge at the twelfth.

This is a wall passed and the family not finished, and the count of
models is the right number to have stayed still.

## The `NaN before any Newton step` row, taken apart (shift 138)

Twenty-two models in the census of shift 202
(`/tmp/m202/census.txt:252`), sorted by the size of the loop that
refused and by what the residual came out as (`/tmp/m204/row22_names.txt`
and the listing beside it):

```text
 loop  value  model
    1  NaN    ModelicaTest...Fittings.TestSharpEdgedOrifice
    2  NaN    ModelicaTest...NewFittings.Orifices.ThickEdgedOrifice
    3  NaN    ModelicaTest...Dissipation.TestCases.PressureLoss.Orifice
    4  -inf   Magnetic.FluxTubes...SaturatedInductor
    6  NaN    Electrical.Analog.Examples.HeatingNPN_NORGate
    6  NaN    Electrical.Analog.Examples.HeatingPNP_NORGate
    9  NaN    MultiBody.Examples.Elementary.DoublePendulum
   12  -inf   Magnetic.QuasiStatic.FluxTubes...NonLinearInductor
   13+ NaN    five Electrical.Machines SMPM/SMR (loops 13 to 47)
   13+ NaN    two FundamentalWave SMPM_Inverter, SMR_Inverter
   22  NaN    MultiBody.Examples.Loops.Fourbar2 (parked)
   26+ NaN    RollingWheel, RollingWheelSet{Driving,Pulling}
   32+ -inf   SolenoidActuator.Comparison{QuasiStatic,PullInStroke}
   33  -inf   MultiBody.Examples.Elementary.ThreeSprings
```

The rows add to exactly twenty-two: 3 orifices + 1 + 2 gates + 1 + 1 +
5 + 2 + 1 + 3 + 2 + 1. The machines are **seven**, not nine - five
under `Electrical.Machines` and two under `FundamentalWave` - and the
two `Heating{NPN,PNP}_NORGate` beside them are transistor examples of
`Electrical.Analog`, which the machine parking of shift 130 does not
cover. Counting them as machines is what made the earlier reading of
this row say nine; the names are in `/tmp/m204/row22_names.txt` and can
be counted.

Five of the twenty-two answer `-inf` rather than `NaN`, and the honest
name of that row is not `1/G_m` but **a division by a quantity whose
start is exactly zero**. Read from the raw listing
(`/tmp/m204/raw_after.txt`), four of the five divide by a conductance -
`r_mFe.G_m` in `SaturatedInductor` and `NonLinearInductor`,
`g_mFeYokeBot.G_m` in the two `SolenoidActuator` comparisons - and the
fifth does not: `ThreeSprings` divides by
`spring2.lineForce.s`, the distance between two points that coincide in
the start position, to get a unit vector
`e_rel_0[1] = r_rel_0[1] / s`. Different quantities, one mechanism: an
equation divides by something the declaration left standing at zero.
So the five belong together under that name, and the parenthetical
`1/G_m` describes four of them.

The rest answer `NaN`, and the row is not one family: the machines are
the machine chain parked since shift 130, the MultiBody three are
geometry, the two NOR gates are live and unparked, and the three
smallest - the orifices - turned out to be something else entirely.
Deducting the parked (Fourbar2, and the seven machines) leaves
**fourteen live** members, not twenty-one.

### The three smallest were the compiler losing a sentence

The probe on the smallest (`/tmp/m204/probe.txt`) prints a start point
that is perfectly finite and a residual that is not, at every value
tried:

```text
newton 0 t=0 |f|=NaN v=[0.0]    f=[NaN]
newton 0 t=0 |f|=NaN v=[1e-6]   f=[NaN]
newton 0 t=0 |f|=NaN v=[1000.0] f=[NaN]
```

A residual built from finite inputs cannot come out NaN, so the fault
was inside something the run walks rather than in the block. It was:
`Modelica.Math.Nonlinear.solveOneNonlinearEquation` checks that the
bracket it was handed contains a root and calls `error(...)` when it
does not (`.msl/Modelica/Math/Nonlinear.mo:671`). A body the run walks
cannot raise - it answers with a number that is not one and leaves its
reason in `Walked::trouble` for whoever evaluated the point to read
back out. The reader for an implicit block refused _before_ reading,
so the library's own sentence was dropped and what came back named the
solver, which is the one place nothing was wrong.

With the reason read out first, the same three models say:

```text
... before any Newton step, because a function it calls could not be
walked: `"The arguments u_min and u_max provided in the function call "`
is a String, and a String has no value a step can carry
```

Which is two findings in one line. The refusal now points at the
library function and at a bracketing that fails, and it also shows
that a message built by concatenation is cut off at its first piece,
because a `String` is refused where the walk wanted a number. The
second half is the next thing to take: a model that explains itself in
prose should have its prose carried whole to the reader.

### The prose carried whole, and the numbers it was holding

Taken in shift 139. Two changes in the walk, both small. A call
standing on its own is read for its value, and
`Modelica.Utilities.Streams.error(text)` is a call standing on its own
whose one argument is a sentence: read as a number, the first literal
piece is a `String` and the walk refuses about the spelling of the
message instead of delivering it. The walk now takes that call the way
it takes an `assert`. And the message is assembled by `prose` rather
than by `message_text`: off the run, a `String(x)` piece can only be a
`?`, because nothing has a value yet, but a walk stands in the frame
that holds the values, so the piece is worked out and the reader gets
the number. A piece that still cannot be worked out keeps the `?`
rather than costing the whole sentence.

What the three orifices say now
(`/tmp/m205/orifice.txt`, from `library check .msl --only
ModelicaTest.Fluid.TestComponents.Fittings.TestSharpEdgedOrifice
--refused`):

```text
... do not bracket the root of the single non-linear equation 0=f(u):
  u_min  = 200
  u_max  = 6000
  fa = f(u_min) = -348110341317103700000
  fb = f(u_max) = -8459081294005737000000000000
fa and fb must have opposite sign which is not the case
```

Which answers the cause-before-consequence question outright, and the
answer is that the fault is ours. The bracket is
`IdealGases.Common.package.mo:367`, `T_h` inverting `h_T` for a
temperature over the validity range of an ideal gas, 200 K to 6000 K -
a bracket the library chose and one that is correct. The residual is
`f(T) = h_T(data, T) - h`, and `h_T` over that range runs in the
hundreds of thousands of joules per kilogram. For `f(200)` to come out
at `-3.5e20`, the `h` handed in has to be about `3.5e20` J/kg, which is
not an enthalpy any fluid has. So the bracket is honest and what was
poured into it is not: an `h` fifteen orders of magnitude too large
reaches `T_h` from the orifice's stream, and no bracketing could
survive it.

The second number says more than the first. `fb = -8.5e27` is not only
huge, it is seven orders of magnitude further from zero than `fa`,
where `h_T(6000) - h` should differ from `h_T(200) - h` by a few
hundred thousand at most. Both ends moving with `T` on that scale means
`h_T` itself is being evaluated with rubbish, not merely being offered
a rubbish `h`. Where the rubbish enters - the orifice's `state_a.h`,
the stream connector, or the medium's own constants - is the next
question, and it is a `why` on the orifice's enthalpy rather than
anything in the solver. Parked here with its address.

The general shape is worth keeping: a compiler that drops a library's
sentence does not merely print a worse message, it loses the
measurement inside the sentence. Three models' worth of `u_min`,
`u_max`, `fa` and `fb` were being computed and thrown away every run,
and reading them back cost one line of walk and answered a question
three shifts of probing had not.

The register before and after says the change was text and nothing
else. Diffed line for line over the run half
(`/tmp/m204/census_after.txt:250` against
`/tmp/m205/census_both.txt`), every row holds its count and its place,
and exactly three rows changed their wording:

```text
-   4 at t = N.N: `X` is a String, and a String has no value a step can carry
+   4 at t = N.N: The arguments u_min and u_max provided in the function call
      solveOneNonlinearEquation(f,u_min,u_max) do ...
-   1 IFN medium function tsat called with too low pressure p = ? Pa <= ? Pa
+   1 IFN medium function tsat called with too low pressure p = -N.N Pa <= ? Pa
-   1 Error in region computation of IFN steam tables(p = ?, h = ?)
+   1 Error in region computation of IFN steam tables(p = N.N, h = N.N)
```

Four models, not three, were losing their sentence, and the shift's
report named them wrongly as "the orifices and one more". Read by name
from the same raw listing, the four are
`Media.Examples.SolveOneNonlinearEquation.Inverse_sh_T`
(`/tmp/m204/raw_after.txt:453`) and
`ModelicaTest.Fluid.TestPipesAndValves.BranchingPipes15/16/17`
(`:505-507`), all four refusing directly, `at t = 0: ... is a String`
with no loop before it. The three orifices (`:470`, `:475`, `:490`)
stand in row 22 with `of algebraic loop ... before any Newton step` in
front, so the bracket wall holds **seven** models: three orifices plus
these four. The count of four was right and the names were not, which
is the second shift running that a row was named by its number instead
of by its list.

Two further models were printing a question mark where
the run knew the number: the IF97 steam tables now say which pressure
was too low and with what enthalpy. The `?` that survives in the
`tsat` row is the triple-point constant, which is a parameter rather
than anything the frame holds, and it keeps its question mark exactly
as the rule says it should. No count moved anywhere, which is the
right outcome for a change that alters what a refusal says and not
which models refuse.

## The `singular Jacobian in algebraic loop` row, taken apart (shift 139)

Eighteen models, read by name from the same raw listing
(`/tmp/m204/raw_after.txt`, the lines that say `built`). The file holds
nineteen occurrences of the words and the nineteenth is not a model: it
is the counter's own summary line at `raw_after.txt:550`, `4 singular
Jacobian in algebraic loop [...]`, count and text together. Counted by
`built`, the total is eighteen with no remainder.

Sorted by the size of the loop that went singular:

```text
 loop  model
    6  Fluid.Examples.DrumBoiler.DrumBoiler
    6  ModelicaTest...TestPipesAndValves.BranchingPipes14
    9  Thermal.FluidHeatFlow.Examples.PumpAndValve
   12  Electrical.QuasiStatic.SinglePhase.Examples.Rectifier
   12  ModelicaTest...TestComponents.Vessels.TestInitialization
   17  ModelicaTest...TestPipesAndValves.BranchingPipes2
   19  Electrical.Polyphase.Examples.PolyphaseRectifier
   19  ModelicaTest...TestPipesAndValves.BranchingPipes1
   19  ModelicaTest...TestPipesAndValves.BranchingPipes12
   20  Electrical.Machines...SynchronousMachines.SMEE_LoadDump
   22  Electrical.Machines...SynchronousMachines.SMEE_Rectifier
   23  FundamentalWave...SynchronousMachines.SMEE_Rectifier
   29  FundamentalWave...SynchronousMachines.SMEE_LoadDump
   46  FundamentalWave...ComparisonPolyphase.SMEE_Generator_Polyphase
   48  FundamentalWave...ComparisonPolyphase.SMPM_Inverter_Polyphase
   48  FundamentalWave...ComparisonPolyphase.SMR_Inverter_Polyphase
   76  FundamentalWave...ComparisonPolyphase.IMC_DOL_Polyphase
  103  ModelicaTest...TestComponents.Pipes.DynamicPipesAndFittings
```

Cut by what is parked, the eighteen fall into three groups and the cut
is exclusive:

```text
 8  machines (SMEE/SMPM/SMR/IMC), parked since shift 130:
    two under Electrical.Machines, six under FundamentalWave
 2  rectifiers with no machine in them: PolyphaseRectifier and
    the QuasiStatic SinglePhase Rectifier - diodes in a loop,
    `idealDiode.s` throughout
 8  fluid: DrumBoiler, PumpAndValve, TestInitialization,
    BranchingPipes{1,12,14,2}, DynamicPipesAndFittings
```

The machine eight of this row and the machine seven of row 22 are the
same parking and not the same models: row 22 holds `SMPM_Braking`,
`SMPM_Inverter`, `SMPM_VoltageSource`, `SMR_DOL`, `SMR_Inverter` and
two `FundamentalWave` inverters, while this row holds the `SMEE_*` pair
and the whole `ComparisonPolyphase` group. Two rows of one parked
family, not one row counted twice.

None of the eighteen are migrants from the `diverged` row of shift 137.
The six that still say `diverged` are `TestWaterPumpDefault{CV,LV}`,
`TestTemperature1`, `BranchingPipes4`, `SeriesPipes{1,2}`, and no name
appears in both lists: the retreat from the edge did not walk anybody
into a singular Jacobian. The counts also did not move - 18 and 6 in
both `/tmp/m204/census.txt:253,261` and `census_after.txt:253,261`.

So the live count of this row is **ten** (18 minus the 8 parked
machines), against **fourteen** live in row 22. Row 22 remains the top
of the queue, and its three smallest members - the orifices - are the
only ones of either row that a single reading has already moved.

Two of the ten are worth naming as the small end: `DrumBoiler` and
`BranchingPipes14` each go singular on a loop of **six**, which is the
smallest singular loop in the corpus and the natural place to ask what
the tearing chose. Both are fluid, and four of the six unknowns in
`BranchingPipes14` are pressures - `pipe{1,2,3}.mediums[1].p` and
`junctionIdeal.medium.p` - which is a junction's pressure written three
times over and a candidate for a loop that is singular because it is
genuinely rank-deficient rather than because the values are bad.

## The bracket wall was a shifted seat (shift 140)

The wall seven models stood at was not about brackets at all. The
probe on the smallest of them, `Inverse_sh_T`
(`/tmp/m206/inverse_sh_T_fable.txt`), printed `u_min = 200`,
`u_max = 6000`, `fa = -7.07e15`, `fb = -5.73e21` - a specific entropy
of air fifteen orders above the few thousand joules per kilogram-kelvin
it is owed, so the bracket was innocent and the function was producing
rubbish.

Where the rubbish came from is a seat nobody filled. A receiver
specialized for a handed-over function takes ordinary numbers where
the function input was, and the fields of a record filled in at the
hand-over are appended after everything the call wrote. Between the
two sits any input the call left to its own default. Brent's method
declares `tolerance` and no medium writes it, so the NASA gas arrived
one seat early: the molar mass read as the tolerance, `Tlimit` read as
a coefficient, every coefficient read as its neighbour.

The instrument that showed it was the pair of field lists printed side
by side - the hand-over road writing twenty-three flat names and the
call carrying twenty-two values into twenty-three seats. Neither list
was wrong; the gap between them was.

Two things are worth keeping from how this was found. The synthetic
models were all green: six of them, each closer to the real shape than
the last, and not one reproduced the fault, because the seat only goes
missing when the receiver declares an input the call does not write -
which is exactly the detail a hand-written reproduction supplies
without thinking. What found it was printing the real specialized body
and reading the argument list against the declared inputs. And the
fault was silent: no refusal anywhere, a wrong number offered as a
right one, and the refusal that did eventually appear named the
bracket, which is to say it named the wrong layer. The census could
never have shown this - the row it filled was about brackets.

Three models arrive, by name from `/tmp/m206/corpus_after.txt` against
`/tmp/m204/raw_after.txt`: `SolveOneNonlinearEquation.Inverse_sh_T`,
`Dissipation.TestCases.PressureLoss.Orifice` and
`NewFittings.Orifices.ThickEdgedOrifice`. The corpus goes 868 / 528 to
868 / 531 and runnable 753 / 496 to 753 / 499.

`BranchingPipes15-17` do not pass this wall. That sentence stood here
for a shift as a guess, referred to a file of run names which by its
nature could hold no refusal, and the guess was wrong. Measured
instead with `--only` on one of them
(`/tmp/m207/bp15_fable.txt`), the refusal is the same bracket wall,
word for word. What changed is what stands inside it:

```text
Inverse_sh_T, before:      fa = -7.07e15      fb = -5.73e21
BranchingPipes15, now:     fa = +113558.53    fb = +7431546.69
```

The rubbish is gone - these are honest magnitudes of an enthalpy, so
the seat the fix filled was filled here too. The bracket `[200, 6000]`
still holds no root, and now for a reason that is about the physics
rather than about the compiler: both ends are positive, so the
temperature the model asks for lies below 200 K, outside the bracket
the standard library wrote. Why the model asks for that is the open
question - either the pressure or the enthalpy arriving from up the
chain is not what it should be, or the model honestly starts in a
state the MSL bracket does not cover. The three models are one storey
up in the same room, not out of it.

## The census after the seat was filled, read against the one before (shift 141)

`scripts/refusals.sh .msl both` into `/tmp/m207/census.txt`, held line
for line against `/tmp/m205/census_both.txt`. The header goes
868 / 528 to 868 / 531 and runnable 753 / 496 to 753 / 499.

The flatten half is **identical line for line** - the whole 73-row
block diffs to nothing. Everything that moved moved in the run half,
and the diff there is five lines:

```text
  `X` of algebraic loop           22 -> 20
  unknown variable `X`             6 ->  7
  u_min and u_max ... bracket      4 ->  3
  cannot evaluate parameters [pipeAV_B.T_start = ...solveOneNonlinear...]  1 -> 0
```

Read by name from `/tmp/m207/raw_before.txt` against
`/tmp/m207/raw_after.txt`, which do contain refusals, unlike the file
the previous shift cited:

- the three arrivals are `Inverse_sh_T`, `PressureLoss.Orifice`,
  `NewFittings.Orifices.ThickEdgedOrifice`, and there are no losses.
  512 statuses before, 509 after, the difference being exactly those
  three leaving the refused list;
- the `u_min` row did **not** empty. One of its four was
  `Inverse_sh_T` and it left; the remaining three are
  `BranchingPipes15/16/17`, at the same wall with different numbers,
  as recorded above. So this is a second missing input by another
  road only in the sense that it is the same road with the mud
  cleared: the arithmetic is now sound and the bracket is genuinely
  wrong for the state;
- `unknown variable` grew by one, and it is
  `DynamicPipeEnergyConservationCheck2`, unable to find `IN_con.a`,
  joining `NewFittings.GenericResistances.VolumeFlowRate`
  which already stood there. That is a model travelling from
  somewhere else to this row, which is the usual shape of a row
  growing after a wall falls.

Worth stating because it is the negative result: the row that grew is
one model, not seven. A wall that let three models through moved
almost nothing else, and the flatten half did not stir at all. The
change was narrow, and the census is what says so.

## Eight registers behind a `break` a model's algorithm cannot decide (shift 141, parked)

The top row of the flatten half that is one family is eight models,
all of `Electrical.Digital.Examples`, counted from
`/tmp/m207/raw_after.txt`: `DFFREG`, `DFFREGL`, `DFFREGSRH`,
`DFFREGSRL`, `DLATREG`, `DLATREGL`, `DLATREGSRH`, `DLATREGSRL`. All
eight refuse with "a branch holding `break` or `return` needs a
condition the compiler can decide", and it is one register component
behind all of them.

The shape, from `.msl/Modelica/Electrical/Digital.mo:4449` onward: a
`for i in 1:n loop` whose body is an `if` on `reset_flag`, itself read
from `ResetMap[reset]` where `reset` is a simulated digital signal.
Inside the branch sits `break`. Nothing before the run can say which
branch is taken, so `one_if_statement`
(`crates/oxidelica-parser/src/flatten/statements.rs:916`) takes the
`has_flow_control` road, which demands a decidable condition, and
refuses.

What makes this dear rather than cheap is where the `break` lives.
The compiler already has an escape hatch for exactly this refusal:
leave the call standing and let the run walk the body, which
`inlining.rs:72` and `equations.rs:920` both do when
`UNDECIDABLE_LEAVING` comes back from a _function_. Here the
algorithm section belongs to a **model** - `model DFFR`, with
`algorithm` at the top level of the component - and a model's
algorithm has no call to leave standing. There is nothing to hand to
the run.

So the fix is not a door opened in the existing hatch. Either the
`break` is turned into predication - every statement after it in the
loop body guarded by a flag the run computes, which is a real
transformation of the section and changes what the merge at the end of
`one_if_statement` must do - or a model's algorithm section grows the
ability to stand for the run the way a function body can. Both are
architectural, and neither is a shift's work with the numbers moved at
the end of it. Parked with the map, and the eight names are the
measure of whether it was worth taking.

## What guards a division by a vector's own length (shift 140, parked)

`ThreeSprings` refuses with `der(body1.Q[3]) = NaN` before any Newton
step, on a loop of thirty-three holding `e_rel_0 = r_rel_0/s`. The
library's guard is real and it works here:
`Interfaces/LineForceBase.mo:45` writes
`s = Frames.Internal.maxWithoutEvent(length, s_small)` with
`s_small = 1e-10`, and asked directly
(`/tmp/m206/mwe.mo`) the compiler answers `s = 1e-10` and `e = 0` for
a zero-length vector. So the NaN is not a division by zero at this
line, and the guard is not what failed. The quaternion derivative is
where to look next, not the line force.

Measured again this shift (`/tmp/m207/threesprings_full.txt`), the
`der(body1.Q[3]) = NaN` wording is gone. What comes back now is
"`spring2.lineForce.e_rel_0[1] = spring2.lineForce.r_rel_0[1] /
spring2.lineForce.s` of algebraic loop", over a loop of thirty-three
naming the three springs' `e_rel_0`, `r_rel_0`, `length` and `s`
together with `body1.a_0`. The quaternion is not in the refusal at
all. So the sentence above pointed at `der(Q)` and the instrument now
points elsewhere: the same division, but named as a loop the solver
cannot enter rather than as a value that came out NaN. Still parked,
and the address has changed - this is one of the `of algebraic loop`
row, not a NaN of its own, so it belongs with that queue and not with
the guard.

## The twenty of `of algebraic loop`, and the one that asked for water (shift 142)

The row read by name off `/tmp/m207/raw_after.txt`, twenty models
counted by the `built` column so that the instrument's own summary line at 537 -
a count and a text, not a model - does not join them:

```text
 7  machines        airGap.RotationMatrix[1,1] = cos(gamma)
 4  FluxTubes       R_m = 1 / G_m
 4  MultiBody       403, 411, 412, 413
 1  orifice         473, dp_fg through an if
 4  parked          HeatingNPN, HeatingPNP, ThreeSprings, Fourbar2
--
20
```

The seven machines are covered by the map at `The air-gap family is
one layer`; sixteen are live, and the four FluxTubes were the narrowest.

What was behind them is not a loop at all. `R_m = 1/G_m` came out
`-inf` because `G_m` was a permeance built from a relative
permeability of one, where the electric sheet the model names states
twelve hundred. The material arrives as `material =
Material.SoftMagnetic.ElectricSheet.M350_50A()`, a constructor call
with no arguments on a record that states its coefficients on its
`extends`, and two layers each dropped half of that:

- `record_components` copied the base's fields and not what the
  `extends` said about them, so the field list carried the base's
  placeholder `mu_i = 1` rather than the sheet's 1210;
- the arity check in `expand_call` counted a call with no arguments
  against the field list and refused it as a call short of its arity.
  The refusal is swallowed where a parameter binding is worked out, so
  the value was dropped without a word and the declared type's own
  defaults stood in its place.

Either alone leaves the wrong number, which is why the test
(`an_empty_constructor_takes_what_the_extends_said`) was watched to go
red with each half removed in turn.

This is the breed the notes already name twice - a wrong number given
quietly, and a reading that cannot see which writer produced the
value. It was not confined to magnetism. Every medium of
`Thermal.FluidHeatFlow` is written this way, and before the change the
whole library computed water as a fluid of density one and heat
capacity one.

The measurement, and it is a loss on the count. The census
(`/tmp/m208/census_after.txt` against `/tmp/m207/census.txt`) moved
one row: `the equations of algebraic loop [...] do not mention` 23 to 24. The one model is
`Modelica.Thermal.FluidHeatFlow.Examples.WaterPump`
(`/tmp/m208/raw_after.txt:289`), and it is a model that was running on
the wrong number: it asks for `Media.Water()` and had been given ones.
With water's figures it stands at the same parked `do not mention`
wall as `PumpAndValve` and `ParallelPumpDropOut`, its two siblings.
Its siblings that still run - `SimpleCooling` and the rest - name
`Media.Medium()` outright, whose properties really are one, so they
were never wrong and did not move.

The four FluxTubes did not move either: with the sheet's coefficients
reaching them the loop is still refused, which is a second wall behind
the first. So this change is one that empties a cause rather than one
that adds a model, and it bought a correct number across two libraries
at the price of one model that had been right by accident.

Floors from `/tmp/m208/floor.txt`: 2671 / 868 / 530 / 753 / 498, the
two run floors down by exactly the one model, with the arithmetic
written beside them in `scripts/library_floor.sh`.

### How far the wrong number actually reached, measured rather than assumed

The reading this change first invited was that every model of
`FluidHeatFlow` had been computing water as a fluid of density one, so
that the one model lost was bought with nine corrected. Ten models of
the library run (`/tmp/m206/ran_after.txt`): `IndirectCooling`,
`OneMass`, `ParallelCooling`, `PumpDropOut`, `SimpleCooling`,
`TestOpenTank`, `TwoMass`, `TwoTanks`, `Utilities.DoubleRamp` and
`WaterPump`.

Put to the instrument, that reading is wrong, and the way it is wrong
is the interesting half. Nine of the ten name `Media.Medium()` - the
base record itself, whose properties really are one apiece, because it
is a placeholder a user is meant to replace. Only `WaterPump` writes
`Media.Water()`. The properties of all nine were probed either side of
one binary (`/tmp/m208/nine_before.txt` against
`/tmp/m208/nine_after.txt`, thirty-six readings each) and the two files
are identical: not one number of the nine moved.

So the honest account of this change in `FluidHeatFlow` is one model,
not ten. The wrong number was real and the bug was real - `Water()`
came back as ones - but it reached exactly the one model that asked
for water, and the model that asked for water was the model that
stopped running. The nine were never wrong to correct.

This is worth writing down because the generous reading was the
plausible one: the mechanism is general, the library is written in the
idiom throughout, and "every model of FluidHeatFlow" follows from both
without a measurement. The measurement took one binary and two
minutes. A reading that makes a change look better is the one to
instrument first.

Where the wrong number did reach breadth is magnetism, which is where
it was found: every `Material.SoftMagnetic` record states its
coefficients on an `extends` and every user of one calls it with empty
parentheses.

### The floor counts models that returned, not models that were right

`WaterPump` is the second recorded case of a model sitting in the run
floor on an answer nobody had checked. The floor asks whether a model
returned without an error - `Answer::Ran` is an `Ok(())` - and that is
a different question from whether what it computed was the model the
library wrote. `WaterPump` asked for water and was handed a fluid of
density one, and the count was as pleased with that as with anything.

The first case is the magnetic machines of shift 132, and it is worth
stating in its own terms rather than stretched to fit this one. There
the question was the Jacobian's difference step, and the measurement
(quoted above) came back mixed: two of three machines gave numbers
that did not depend on the step at all, so they were answers and not
noise, and only `IMC_YD` sat in the floor on a coin toss - converging
at one step and not at another. So the precedent is one model of
uncertain footing rather than three wrong ones.

Two cases, then, and they differ in kind: one model computing a
definite wrong number, and one converging on the edge. What they share
is the property of the instrument, and that is the part worth keeping:
a count of models that returned cannot tell a right answer from a
wrong one, so a change that corrects a number and costs a model reads
on every instrument this project has as a regression. The floors
catching the fall is the instrument working; the arithmetic written
beside the number, naming the model and why, is the only thing that
stops the fall being misread a week later. That is the second time
that comment has been what carried the meaning, after the carving-out
of the heavy models, and it is why this note records a case with only
a single model behind it.

### Reach is counted from who called the broken thing, never from a package name

Both readings corrected above went wrong the same way, and the way is
worth naming on its own. The reach of this fix was first put at every
model of `Thermal.FluidHeatFlow`, because the package holds ten
running models and the idiom that broke is written throughout it. That
is an inference from a name: the package is not the medium, and nine
of the ten name a base record whose properties genuinely are one. The
same shape produced "three magnetic machines" for a case that was one
machine on a coin toss.

The rule that follows costs almost nothing to keep. Reach is the set
of models that actually called the broken thing, and it is read by
comparing numbers either side of one binary rather than by reading a
directory listing. Here that was thirty-six readings and two minutes
(`/tmp/m208/nine_before.txt`, `/tmp/m208/nine_after.txt`), against a
claim of nine corrected models that would otherwise have gone into a
commit message as fact.

This is the note about a name shortened to its tail, seen from a third
side: there the guess was that a name's last part identifies its
owner, here that a name's first part identifies its users. A name is
not a measurement in either direction.
