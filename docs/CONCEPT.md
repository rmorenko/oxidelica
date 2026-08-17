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
2. **Reference checking.** Every model in the test suite is also run in OpenModelica (in CI, via Docker) — results are compared numerically.
3. **Every milestone is useful.** Not a "big bang in three years" but a usable tool after each milestone.
4. **Single binary.** Simulation via interpretation/JIT, no system C compiler involved.

## Roadmap

| Milestone                         | Scope                                                          | Definition of done                                             |
| --------------------------------- | -------------------------------------------------------------- | -------------------------------------------------------------- |
| **M0. Spike**                     | Parser for a language slice + RK4, CLI                         | `der(x)=-x` and a pendulum simulate, error vs analytics < 1e-6 |
| **M1. ODE subset**                | Full expressions, parameters, algebraic equations, adaptive RK | 10 textbook models match OpenModelica                          |
| **M2. Components**                | Classes, inheritance, connect, flattening                      | RC circuit and mass-spring built from components               |
| **M3. DAE** (done)                | Pantelides, dummy derivatives, tearing, BDF                    | Cartesian pendulum (index-3)                                   |
| **M4. Events** (done)             | when/if equations, zero-crossing, reinit                       | Bouncing ball, diode                                           |
| **M5. Arrays & functions** (done) | Arrays, for-equations, functions, records                      | Discretized heat conduction                                    |
| **M6. MSL core** (partial)        | Blocks, Electrical.Analog, Mechanics.Rotational                | A library in MSL layout ships; MSL syntax parses               |
| **M7. GUI** (done)                | Bevy: diagram editor, plotting                                 | Build and simulate a model with the mouse                      |
| **M8. 3D** (done)                 | MultiBody visualization, animation                             | Double pendulum spins in 3D                                    |

**The IDE track runs in parallel with the language track from the start** (decision of 2026-08-15): v0 — a Bevy window with a code editor, a run button, and plots (shipped with M0); then incrementally — background simulation, syntax highlighting, the diagram editor (M7) and the 3D scene (M8) in the same app. The language track advances at its own pace: M1, M2, M3…

## Tech stack

- **Core**: Rust, a workspace of crates: `parser` → `flatten` → `dae` → `sim` → `runtime`.
- **GUI/3D**: Bevy + bevy_egui (panels), wgpu rendering. Plus a WASM build of the core for the web (later).
- **Tests**: cargo test + reference runs against OpenModelica in Docker (CI).

## Key risks

1. **DAE index reduction (M3)** — done: Pantelides with dummy derivatives, tearing and an implicit BDF solver. The state selection is static (chosen by numerical pivoting at the initial point); models that need it to change mid-run — a pendulum swinging full circle — are the known limit.
2. **The long tail of MSL semantics (M6)** — partially addressed: a standard library in MSL layout ships with the tool, and MSL syntax (packages, dotted names, type aliases, partial classes, imports, `within`, graphical annotations, `assert`, `noEvent`) parses. Real MSL files still need `replaceable`/`redeclare`, `inner`/`outer`, enumerations and conditional components.
3. **Bevy maturity as a UI framework (M7)** — a diagram editor on ECS is nontrivial; mitigate: all "office" UI in egui, Bevy owns the canvas and 3D.

## Spike (M0) — definition

Language slice: `model … end`, `Real`/`parameter Real` declarations with the `start` attribute, an `equation` section, `der()`, arithmetic, elementary functions, `time`, `annotation(experiment(StopTime=…))`. Fixed-step RK4 solver, CSV output. Success criterion: exponential decay and a pendulum match analytics; the spike code becomes the foundation of M1, not a throwaway.
