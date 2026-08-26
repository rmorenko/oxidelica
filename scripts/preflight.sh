#!/usr/bin/env bash
#
# Everything CI checks, before the push rather than after it.
#
# CI runs six jobs, and it is easy to run the two that are on the mind
# at the time - the tests and the library floor - push, and find out
# from the machine that a linter or the coverage threshold had
# something to say. That round trip costs several minutes and a red
# commit on the branch. This runs the same checks here.
#
# What is skipped and why: the three-platform matrix, since this is one
# machine; the advisories job, which asks about the world rather than
# about the commit and is on a weekly schedule of its own. Everything
# else is the same command CI runs, so a pass here means a pass there
# unless the difference is the platform itself.
#
# Usage: scripts/preflight.sh [--quick] [<library directory>]
#
#   --quick  the fast checks only - formatting, clippy, tests, examples.
#            Coverage and the linters are the slow ones, and while a
#            change is still moving they are noise; run the whole thing
#            before the push.
#
# The library directory defaults to where `library add` puts it. Where
# there is none the floor check says so and is skipped rather than
# failing: the floors are about a library this machine may not have.
set -uo pipefail
cd "$(dirname "$0")/.."

quick=0
library=""
for argument in "$@"; do
  case "$argument" in
    --quick) quick=1 ;;
    -*) echo "unknown option: $argument" >&2; exit 2 ;;
    *) library="$argument" ;;
  esac
done

if [ -z "$library" ]; then
  library="${XDG_DATA_HOME:-$HOME/.local/share}/oxidelica/libraries/Modelica"
fi

# Every failure is reported at the end rather than stopping the run.
# A push is held up by the whole list, and finding out about one thing
# at a time is what makes this slow enough to skip.
failures=()
skipped=()

step() {
  local name="$1"
  shift
  printf '\n\033[1m== %s\033[0m\n' "$name"
  if "$@"; then
    printf '\033[32mok\033[0m: %s\n' "$name"
  else
    printf '\033[31mFAILED\033[0m: %s\n' "$name"
    failures+=("$name")
  fi
}

# A linter this machine does not have is not a failure of the commit.
# CI has them all; here they are reported as unchecked so the gap is
# visible rather than silently passing.
optional_step() {
  local name="$1"
  local tool="$2"
  shift 2
  if ! command -v "$tool" > /dev/null 2>&1; then
    printf '\n\033[1m== %s\033[0m\n\033[33mskipped\033[0m: `%s` is not installed\n' "$name" "$tool"
    skipped+=("$name")
    return
  fi
  step "$name" "$@"
}

examples_all_simulate() {
  cargo build --release -p oxidelica-cli || return 1
  local status=0
  for model in examples/*.mo; do
    if ! ./target/release/oxidelica simulate "$model" > /dev/null 2>&1; then
      echo "FAILED: $model"
      status=1
    fi
  done
  return $status
}

library_floor() {
  if [ ! -d "$library" ]; then
    return 0
  fi
  cargo build --release -p oxidelica-cli || return 1
  ./scripts/library_floor.sh "$library"
}

step "Formatting" cargo fmt --all -- --check
step "Clippy" cargo clippy --workspace --all-targets -- -D warnings
step "Tests" cargo test --workspace
step "The examples all simulate" examples_all_simulate

if [ -d "$library" ]; then
  step "The standard library still reads" library_floor
else
  printf '\n\033[1m== The standard library still reads\033[0m\n'
  printf '\033[33mskipped\033[0m: no library at %s\n' "$library"
  skipped+=("The standard library still reads")
fi

if [ "$quick" -eq 0 ]; then
  optional_step "Coverage, the threshold is 95% of lines" cargo-llvm-cov \
    ./scripts/coverage.sh --summary-only
  optional_step "TOML" taplo taplo fmt --check --diff
  optional_step "Spelling" typos typos
  # The workflow checks the prose as well as the code, and a green
  # commit here that went red there was a formatter this script never
  # ran: the same versions the workflow names, so what passes here
  # passes there.
  optional_step "Markdown, JSON and YAML" npx \
    npx --yes prettier@3.9.6 --check "**/*.{md,json,yaml,yml}" --log-level warn
  # The same files CI lints, which is the ones git tracks: the running
  # note left for fable is ignored by git and is a journal rather than
  # a document - one heading per round, the same headings every round -
  # so linting it here made the preflight red for something CI never
  # sees, and a check that is always red is a check nobody runs.
  optional_step "Markdown style" npx \
    npx --yes markdownlint-cli2@0.23.2 "**/*.md" "!target" "!QUESTION_FOR_FABLE.md"
  step "No Cyrillic outside the files that may hold it" python3 scripts/check_cyrillic.py
  optional_step "Unused dependencies" cargo-machete cargo machete
  step "Documentation builds, and every public item has some" \
    env RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps
fi

printf '\n\033[1m== summary ==\033[0m\n'
for name in "${skipped[@]:-}"; do
  [ -n "$name" ] && printf '\033[33munchecked\033[0m: %s\n' "$name"
done
if [ "${#failures[@]}" -eq 0 ]; then
  if [ "$quick" -eq 1 ]; then
    printf '\033[32mthe fast checks pass\033[0m - run without --quick before pushing\n'
  else
    printf '\033[32meverything CI checks on one platform passes\033[0m\n'
  fi
  exit 0
fi
for name in "${failures[@]}"; do
  printf '\033[31mfailed\033[0m: %s\n' "$name"
done
exit 1
