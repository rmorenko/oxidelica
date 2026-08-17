#!/usr/bin/env bash
# Покрытие ядра тестами; порог проекта — 95% строк.
# GUI-крейт (oxidelica-ide) исключён: event loop Bevy юнит-тестами не покрывается.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo llvm-cov -p oxidelica-parser -p oxidelica-sim -p oxidelica-cli \
  --fail-under-lines 95 "$@"
