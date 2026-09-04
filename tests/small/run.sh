#!/usr/bin/env bash
# Flatten every small model and say which ones the compiler refuses.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
oxidelica="$root/target/release/oxidelica"
[ -x "$oxidelica" ] || { echo "build first: cargo build --release"; exit 2; }

pattern="${1:-}"
refused=0
total=0
for model in "$here"/*.mo; do
  name="$(basename "$model" .mo)"
  [ -n "$pattern" ] && [[ "$name" != *"$pattern"* ]] && continue
  total=$((total + 1))
  # `why` on a name no model has flattens it and reports what stopped.
  why="$("$oxidelica" why "$model" __nothing__ 2>&1)"
  if printf '%s' "$why" | grep -q "refused the model\|^error"; then
    refused=$((refused + 1))
    printf '  refused  %s\n' "$name"
    printf '%s' "$why" | grep -m1 "refused the model:\|^error" | sed 's/^ */           /'
  else
    printf '  flat     %s\n' "$name"
  fi
done
printf '\n%d model(s), %d refused\n' "$total" "$refused"
