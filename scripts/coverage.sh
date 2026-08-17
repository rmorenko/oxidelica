#!/usr/bin/env bash
# Core test coverage; the project threshold is 95% of lines.
# The GUI crate (oxidelica-ide) is excluded: the Bevy event loop is not
# unit-testable.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo llvm-cov -p oxidelica-parser -p oxidelica-sim -p oxidelica-cli \
  --fail-under-lines 95 "$@"
