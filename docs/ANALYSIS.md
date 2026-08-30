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
