# Oxidelica

A modern cross-platform physical modeling environment in Rust, compatible with the Modelica language. The goal is the full language specification, a modern GUI and 3D visualization on Bevy. Details: [docs/CONCEPT.md](docs/CONCEPT.md) ([Russian](docs/CONCEPT.ru.md)).

Russian version of this file: [README.ru.md](README.ru.md).

**Status: M0 (spike)** — a parser for a Modelica slice + an RK4 simulator + CLI + the IDE v0.

## Quick start

```bash
cargo run -p oxidelica-cli -- simulate examples/pendulum.mo -o pendulum.csv
cargo run -p oxidelica-cli -- parse examples/decay.mo
cargo run -p oxidelica-ide
cargo test
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

- `crates/oxidelica-parser` — lexer, AST, recursive descent over the Modelica slice
- `crates/oxidelica-sim` — compilation of a flat model into explicit ODEs + RK4
- `crates/oxidelica-cli` — the `oxidelica` binary (simulate / parse)
- `crates/oxidelica-ide` — the Bevy + egui IDE (menu, editor, plots, EN/RU, themes)
- `docs/` — concept and roadmap (M0…M8)
- `examples/` — models in honest Modelica syntax (they open in OpenModelica too)
