# Oxidelica

A modern cross-platform physical modeling environment in Rust, compatible with the Modelica language. The goal is the full language specification, a modern GUI and 3D visualization on Bevy. Details: [docs/CONCEPT.md](docs/CONCEPT.md) ([Russian](docs/CONCEPT.ru.md)); what is and is not covered of the specification: [docs/COMPLIANCE.md](docs/COMPLIANCE.md).

Russian version of this file: [README.ru.md](README.ru.md).

**Status: the M0…M8 roadmap is complete** — a Modelica compiler (packages, inheritance, connections, DAE index reduction, events, arrays, functions, and the MSL structure: `replaceable`/`redeclare`, `inner`/`outer`, enumerations, conditional components), adaptive and stiff solvers, and an IDE with an editor, plots, a diagram editor and a 3D scene.

## Quick start

```bash
cargo run -p oxidelica-cli -- simulate examples/pendulum.mo -o pendulum.csv
cargo run -p oxidelica-cli -- parse examples/decay.mo
cargo run -p oxidelica-ide
cargo test
```

## Platforms

The point of the project is one native binary on all three desktops, so
every commit is built and tested on macOS, Linux and Windows by
[the CI workflow](.github/workflows/ci.yml); tagging `v*` builds the
archives for the three targets. What has been verified locally so far:
macOS natively; Linux in Docker, where the core passes its tests and the
whole workspace is clippy-clean; and Windows by cross-compiling the whole
workspace, GUI included, into real `.exe` files and running the entire
test suite on them under Wine. The results agree across platforms to
1e-9, which is the event-location tolerance rather than a difference in
the answers.

A model does not have to live in this folder: the standard library is
looked for as `lib` next to the model, next to the working directory or
next to the binary, and `OXIDELICA_LIB` names it outright.

Linux needs the libraries Bevy links against — on Debian and Ubuntu
`pkg-config libasound2-dev libudev-dev`; the Windows cross-build needs
`mingw-w64`, and running its binaries needs `wine`. Both checks run from
a Mac without leaving the repository:

```bash
make linux-check
make windows-check
```

## Quality

The project threshold is **95% line coverage** for the core (parser, sim, cli; the GUI crate is excluded — the Bevy event loop is not unit-testable). The full pipeline lives in the Makefile:

```bash
make help
make check
make cov-report
```

Language rule: no Cyrillic anywhere except Markdown files with a `.ru.md` suffix and the IDE locale file `locales/ru.conf`; enforced by `make lint-cyrillic`.

## Layout

- `crates/oxidelica-parser` — lexer, AST, recursive descent, instantiation and flattening
- `crates/oxidelica-sim` — symbolic analysis of a flat model (sorting, index reduction, tearing) and the solvers: Dormand-Prince 5(4), BDF 1-5, RK4, with event location
- `crates/oxidelica-cli` — the `oxidelica` binary (simulate / parse)
- `crates/oxidelica-ide` — the Bevy + egui IDE (menu, editor, plots, EN/RU, themes)
- `docs/` — concept and roadmap (M0…M8)
- `examples/` — models in honest Modelica syntax (they open in OpenModelica too)
