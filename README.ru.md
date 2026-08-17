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

## Платформы

Смысл проекта — один нативный бинарник на всех трёх десктопах, поэтому
каждый коммит собирается и тестируется на macOS, Linux и Windows
[рабочим процессом CI](.github/workflows/ci.yml); тег `v*` собирает
архивы под три цели. Что проверено локально: macOS нативно; Linux в
Docker, где ядро проходит тесты, а весь workspace чист по clippy; Windows
— кросс-сборкой всего workspace, включая GUI, в настоящие `.exe` и
прогоном всего набора тестов на них под Wine. Результаты сходятся между
платформами до 1e-9 — это допуск локализации событий, а не расхождение в
ответах.

Модель не обязана лежать в этой папке: стандартная библиотека ищется как
`lib` рядом с моделью, рядом с рабочим каталогом или рядом с бинарником, а
`OXIDELICA_LIB` задаёт её напрямую.

Linux'у нужны библиотеки, с которыми линкуется Bevy: на Debian и Ubuntu
`pkg-config libasound2-dev libudev-dev`; кросс-сборке под Windows нужен
`mingw-w64`, а запуску её бинарников — `wine`. Обе проверки запускаются с
макбука, не выходя из репозитория:

```bash
make linux-check
make windows-check
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
