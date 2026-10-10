#!/usr/bin/env bash
# Which state index reduction demotes, asked of a model that needs the
# standard library and so cannot stand in the test suite.
#
# A change that gives a name a value changes which states reduction
# keeps, and that is what is measured rather than whether a model runs.
# `tests/small/a_derivative_name_weighed_at_the_start.mo` is four
# revolute joints squeezed out of `MechanicalStructure`: its eighty-sixth
# reduction is a tie whose cone reads `der(r1.phi)`, a name the first
# reduction made and nothing gave a start. Weighed with the starts
# alone the cone cannot be read, the tie falls to the order of the walk
# and `r3.w` is demoted. Weighed with what the derivative names work out
# to at the start, the tie keeps only what the constraint determines and
# `b4.body.v_0[2]` is demoted.
#
# Both halves are asked of one binary, with
# OXIDELICA_NO_DERIVATIVES_AT_REDUCTION for the old one, so the witness
# goes red if the change is lost and red if the switch stops giving the
# old choice back. Neither half runs: the model stops at a start a
# divisor reads as zero, which is a different wall, so the exit status
# is not what is checked.
#
#   scripts/victim_witness.sh <library directory> [<oxidelica binary>]
set -euo pipefail

library="${1:?usage: victim_witness.sh <library directory> [<oxidelica binary>]}"
cd "$(dirname "$0")/.."
binary="${2:-./target/release/oxidelica}"
model="tests/small/a_derivative_name_weighed_at_the_start.mo"
[ -d "$library" ] || { echo "WITNESS: no library at $library"; exit 1; }
[ -x "$binary" ] || { echo "WITNESS: no binary at $binary"; exit 1; }
[ -f "$model" ] || { echo "WITNESS: no model at $model"; exit 1; }
library="$(cd "$library" && pwd)"

# The victim of the eighty-sixth reduction, as the probe prints it. The
# run's own refusal is expected and goes to the log, so `|| true` stands
# for the model and not for the pipe: an empty answer below is a
# failure, not a zero.
victim_at() {
  local at="$1" log
  shift
  log="$(env "$@" OXIDELICA_LIB="$library" OXIDELICA_VICTIM_PROBE=1 \
    "$binary" simulate "$model" --stop 0 2>&1 > /dev/null || true)"
  echo "$log" | awk -v at="$at" '
    /^victim-probe: reduction / { r = $3 }
    /^victim-probe:   victim: / && r == at && !seen { print $3; seen = 1 }'
}

status=0
check() {
  local side="$1" at="$2" want="$3" got="$4"
  if [ "$got" = "$want" ]; then
    echo "witness $side: reduction $at demotes $got"
  else
    echo "WITNESS $side: reduction $at demotes '${got}', expected $want"
    status=1
  fi
}
# The weighing at the start is seen only where the tie it decides is
# still open: folding the constant candidates and reading stateSelect
# both decide the same ties earlier, so this pair holds them off.
before=(OXIDELICA_NO_CONSTANT_CANDIDATES=1 OXIDELICA_NO_STATE_SELECT=1)
check "weighed at the start" 86 "b4.body.v_0[2]" "$(victim_at 86 "${before[@]}")"
check "starts alone" 86 "r3.w" \
  "$(victim_at 86 "${before[@]}" OXIDELICA_NO_DERIVATIVES_AT_REDUCTION=1)"
# What the model says about its states decides the tie of reduction 26,
# where the first joint's angle (prefer) weighs nothing against a body's
# position (avoid): read, the position goes and the angle stays a state.
check "stateSelect read" 26 "b2.r_0[1]" "$(victim_at 26 OXIDELICA_UNUSED=1)"
check "stateSelect unread" 26 "r1.phi" "$(victim_at 26 OXIDELICA_NO_STATE_SELECT=1)"
# A preferred position whose velocity is no state is preferred for the
# pair or not at all. Read for the pair, the actuator's first reduction
# demotes the stopper's position; read for the position alone, it keeps
# the position and demotes the armature's, and the run stops at the
# stopper's velocity nothing determines.
model="tests/small/a_preferred_position_whose_velocity_is_no_state.mo"
[ -f "$model" ] || { echo "WITNESS: no model at $model"; exit 1; }
check "half pair unread" 1 "cActuator.armature.stopper_xMax.s_rel" "$(victim_at 1 OXIDELICA_UNUSED=1)"
check "half pair read" 1 "cActuator.armature.mass.s" "$(victim_at 1 OXIDELICA_NO_HALF_PAIRS=1)"
exit "$status"
