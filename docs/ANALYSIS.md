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

So the link is: an `if` equation settled at flatten time keeps its
surviving branch, but that branch is not offered to the clock
partition afterwards. Named, not taken - the shift is spent, and the
same rule applies as to `nXi`: a link half-taken is worse than one
written down.
