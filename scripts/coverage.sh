#!/usr/bin/env bash
# Core test coverage; the project threshold is 95% of lines.
# The GUI crate (oxidelica-ide) is excluded: the Bevy event loop is not
# unit-testable.
#
# The figure reads about a point lower than the suite really reaches.
# Most tests live in tests/, which links the crate as a dependency,
# while a handful of unit tests keep it compiled a second time with
# cfg(test); llvm-cov merges the two profiles, but not perfectly, and
# small functions compiled into both come out under-counted. Nothing is
# untested that used to be tested - the same tests run either way.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo llvm-cov -p oxidelica-parser -p oxidelica-sim -p oxidelica-cli \
  --fail-under-lines 95 "$@"
