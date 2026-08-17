# Oxidelica

Современная кроссплатформенная среда физического моделирования на Rust, совместимая с языком Modelica. Цель — полная спецификация языка, современный GUI и 3D-визуализация на Bevy. Подробности: [docs/CONCEPT.md](docs/CONCEPT.md) ([EN](docs/CONCEPT.en.md)).

**Статус: M0 (спайк)** — парсер среза Modelica + RK4-симулятор + CLI.

## Быстрый старт

```bash
cargo run -p oxidelica-cli -- simulate examples/pendulum.mo -o pendulum.csv
cargo run -p oxidelica-cli -- parse examples/decay.mo
cargo test
```

## Структура

- `crates/oxidelica-parser` — лексер, AST, рекурсивный спуск по срезу Modelica
- `crates/oxidelica-sim` — компиляция плоской модели в явную ОДУ + RK4
- `crates/oxidelica-cli` — бинарник `oxidelica` (simulate / parse)
- `docs/` — концепция и дорожная карта (M0…M8)
- `examples/` — модели на честном синтаксисе Modelica (открываются и в OpenModelica)
