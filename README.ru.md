# Oxidelica

Современная кроссплатформенная среда физического моделирования на Rust, совместимая с языком Modelica. Цель — полная спецификация языка, современный GUI и 3D-визуализация на Bevy. Подробности: [docs/CONCEPT.ru.md](docs/CONCEPT.ru.md) ([English](docs/CONCEPT.md)).

Английская версия этого файла: [README.md](README.md).

**Статус: дорожная карта M0…M8 пройдена** — компилятор Modelica (пакеты, наследование, соединения, приведение индекса ДАУ, события, массивы, функции и структура MSL: `replaceable`/`redeclare`, `inner`/`outer`, перечисления, условные компоненты), адаптивный и жёсткий решатели, IDE с редактором, графиками, схемным редактором и 3D-сценой.

## Быстрый старт

```bash
cargo run -p oxidelica-cli -- simulate examples/pendulum.mo -o pendulum.csv
cargo run -p oxidelica-cli -- parse examples/decay.mo
cargo run -p oxidelica-ide
cargo test
```

## Качество

Порог проекта — **95% строк** покрытия по ядру (parser, sim, cli; GUI-крейт исключён — event loop Bevy юнит-тестами не покрывается). Весь конвейер — в Makefile:

```bash
make help
make check
make cov-report
```

Языковое правило: кириллица допустима только в Markdown-файлах с суффиксом `.ru.md` и в файле локализации IDE `locales/ru.conf`; проверяется `make lint-cyrillic`.

## Структура

- `crates/oxidelica-parser` — лексер, AST, рекурсивный спуск, инстанцирование и раскрытие модели
- `crates/oxidelica-sim` — символьный анализ плоской модели (сортировка, приведение индекса, tearing) и решатели: Dormand-Prince 5(4), BDF 1-5, RK4, с локализацией событий
- `crates/oxidelica-cli` — бинарник `oxidelica` (simulate / parse)
- `crates/oxidelica-ide` — IDE на Bevy + egui (меню, редактор, графики, EN/RU, темы)
- `docs/` — концепция и дорожная карта (M0…M8)
- `examples/` — модели на честном синтаксисе Modelica (открываются и в OpenModelica)
