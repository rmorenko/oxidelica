# Oxidelica — Concept

**A modern cross-platform physical modeling environment in Rust, compatible with the Modelica language.**

## Why

OpenModelica is powerful but operationally heavy: no native macOS build, GUI requires X11/Docker acrobatics, dated interface. Oxidelica is an attempt to build a next-generation modeling environment: a single native binary for Mac/Linux/Windows, a modern interface with 3D visualization built on Bevy, and a Rust core that never invokes a C compiler on the user's machine.

## What it is

- **A Modelica compiler** (goal — the full specification; path — an incrementally growing subset): parser → instantiation & flattening → symbolic DAE analysis (sorting, index reduction, tearing) → executable form.
- **A numeric core**: explicit and implicit ODE/DAE solvers (RK, BDF/IDA-class), events, hybrid systems.
- **An environment**: diagram editor, plotting, 3D animation of MultiBody models — all on Bevy (ECS + wgpu), UI panels on egui.

## Principles

1. **The real language.** No "dialect": every stage parses a subset of honest Modelica; files stay compatible with OpenModelica.
2. **Reference checking.** Every model in the test suite is checked against something independent of the code that produced it: a closed form where one exists (analytic trajectory, steady state, conserved quantity), or a second formulation of the same physics.
3. **Every milestone is useful.** Not a "big bang in three years" but a usable tool after each milestone.
4. **Single binary.** Simulation via interpretation/JIT, no system C compiler involved.

## Roadmap

| Milestone                         | Scope                                                                                 | Definition of done                                               |
| --------------------------------- | ------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| **M0. Spike**                     | Parser for a language slice + RK4, CLI                                                | `der(x)=-x` and a pendulum simulate, error vs analytics < 1e-6   |
| **M1. ODE subset**                | Full expressions, parameters, algebraic equations, adaptive RK                        | 10 textbook models match their closed forms                      |
| **M2. Components**                | Classes, inheritance, connect, flattening                                             | RC circuit and mass-spring built from components                 |
| **M3. DAE** (done)                | Pantelides, dummy derivatives, tearing, BDF                                           | Cartesian pendulum (index-3)                                     |
| **M4. Events** (done)             | when/if equations, zero-crossing, reinit                                              | Bouncing ball, diode                                             |
| **M5. Arrays & functions** (done) | Arrays, for-equations, functions, records                                             | Discretized heat conduction                                      |
| **M6. MSL core** (done)           | Blocks, Electrical.Analog, Mechanics.Rotational                                       | A library in MSL layout ships; MSL structure parses and flattens |
| **M7. GUI** (done)                | Bevy: diagram editor, plotting                                                        | Build and simulate a model with the mouse                        |
| **M8. 3D** (done)                 | MultiBody visualization, animation                                                    | Double pendulum spins in 3D                                      |
| **M9. Discrete layer** (done)     | Discrete variables, `when` equations, `pre`, `sample`, algorithms, `initial equation` | A sampled controller, a thermostat and a steady start run        |

**The IDE track runs in parallel with the language track from the start** (decision of 2026-08-15): v0 — a Bevy window with a code editor, a run button, and plots (shipped with M0); then incrementally — background simulation, syntax highlighting, the diagram editor (M7) and the 3D scene (M8) in the same app. The language track advances at its own pace: M1, M2, M3…

## Tech stack

- **Core**: Rust, a workspace of crates: `parser` → `flatten` → `dae` → `sim` → `runtime`.
- **GUI/3D**: Bevy + bevy_egui (panels), wgpu rendering. Plus a WASM build of the core for the web (later).
- **Tests**: cargo test, with every example checked against a closed form or an equivalent model.

## Key risks

1. **Choosing the solver (M10)** — done: a run starts explicit and watches the product of the step size and the dominant eigenvalue, which the Dormand-Prince stages give away for free. Past the stability limit of the method the step is no longer about accuracy, and the run restarts on the implicit solver. On a heat rod of 200 nodes that is 10.3 s against 0.68 s, with nothing to set by hand; every other example stays explicit.
2. **DAE index reduction (M3)** — done: Pantelides with dummy derivatives, tearing and an implicit BDF solver. The state selection is dynamic: after every accepted step the run watches the sensitivity of each reduced constraint to its demoted state, and when the pivot that chose the states would now choose differently, it compiles itself again at the current point and continues. A Cartesian pendulum going over the top re-selects every quarter turn and agrees with the angle form to 1e-6.
3. **The long tail of MSL semantics (M6)** — closed for the packages the milestone covers. A standard library in MSL layout ships with the tool, and the structure real MSL files are built on works: `replaceable`/`redeclare` checked against `constrainedby`, `inner`/`outer` instances, enumerations, conditional components with the connections to them removed, `if` equations chosen by a structural parameter, declaration equations, nested modifiers reaching a child's attribute, chained type aliases, and the annotations MSL puts on declarations, equations and connections. Still out of scope: class-level redeclaration (`replaceable package Medium = …`) and the descriptive attributes (`unit`, `min`, `max`, `nominal`, `stateSelect`), which are parsed and ignored. Arrays, on the other hand, are values now: literals, whole-array equations, `.+`/`.*`/`./`/`.^`, scalar products, `size`/`sum`/`product`/`min`/`max`, `zeros`/`ones`/`fill`/`linspace`, array starts and bindings, and scalar functions vectorizing over elements - everything expanded into scalars while flattening. Connections take subscripts, run inside `for` loops and pair whole connector arrays element by element. Functions take and return arrays - assigned whole or element by element, called by qualified name - and inline like everything else.
4. **The discrete layer (M9)** — done: variables that change only at events, `when` equations with `elsewhen`, `pre`/`edge`/`change`/`initial()`, the clock of `sample(start, interval)` with the solver stepping exactly onto it, and event iteration so one event can chain several clauses. `algorithm` sections now work in models too — the compiler executes them symbolically into one equation per assigned variable, merging the branches of an `if` into an expression and unrolling a `for`. `initial equation` solves for the state the run starts from by Newton, with the declared `start` values as the guess. Out of scope: `while` in an algorithm (no trip count the compiler can see) and `when` inside an algorithm.
5. **Bevy maturity as a UI framework (M7)** — a diagram editor on ECS is nontrivial; mitigate: all "office" UI in egui, Bevy owns the canvas and 3D.

## Spike (M0) — definition

Language slice: `model … end`, `Real`/`parameter Real` declarations with the `start` attribute, an `equation` section, `der()`, arithmetic, elementary functions, `time`, `annotation(experiment(StopTime=…))`. Fixed-step RK4 solver, CSV output. Success criterion: exponential decay and a pendulum match analytics; the spike code becomes the foundation of M1, not a throwaway.
