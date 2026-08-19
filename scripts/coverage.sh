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
#
# The run is given no library to find. A machine with the standard
# library fetched would otherwise read thousands of files where a bare
# one reads a handful, and the figure would say more about the machine
# than about the tests: the same commit measured three quarters of a
# point higher here than in CI until this was fixed. Tests that want a
# library still make one and point at it themselves.
set -euo pipefail
cd "$(dirname "$0")/.."
nowhere="$(mktemp -d)"
trap 'rm -rf "$nowhere"' EXIT
export XDG_DATA_HOME="$nowhere"
unset OXIDELICA_LIB MODELICAPATH
cargo llvm-cov -p oxidelica-parser -p oxidelica-sim -p oxidelica-cli \
  --fail-under-lines 95 "$@"
