#!/usr/bin/env bash
# A judge of the delta between two prints of the main pass, standing
# beside the observer in ci.yml:
#   delta_guard.sh <previous green main.txt> <current main.txt>
# It judges the run/flatten quotient of the delta (run ratio over
# flatten ratio, from ratio_delta.sh), which cancels the runner's
# weather the way the band's ratio does. Limit: twice the worst quiet
# pair of the runner's history (12.7%), with the clean pairs of one
# compiler inside half of it (docs/ANALYSIS.md). Main pass only: the
# dear pass reads 1.154 on equal work over fourteen models. It sees a
# step once, since the next green print becomes the base; the band in
# library_floor.sh keeps seeing the level. No base = not judged, green.
set -uo pipefail
hi=${DELTA_GUARD_HIGH:-1.25}; lo=${DELTA_GUARD_LOW:-0.75}
if [[ $# -ne 2 || ! -s "${1:-}" || ! -s "${2:-}" ]]; then
  echo "delta guard: no base print, not judged"; exit 0
fi
line=$("$(dirname "$0")/ratio_delta.sh" "$1" "$2" | grep '^delta: flatten') || {
  echo "delta guard: delta not taken, not judged"; exit 0; }
r=$(echo "$line" | tr -d 's();' | awk '{printf "%.3f", $11/$6}')
if awk -v r="$r" -v lo="$lo" -v hi="$hi" 'BEGIN{exit !(r>=lo && r<=hi)}'; then
  echo "delta guard: run/flatten delta $r inside $lo..$hi"
else
  echo "delta guard: run/flatten delta $r OUTSIDE $lo..$hi"; exit 1
fi
