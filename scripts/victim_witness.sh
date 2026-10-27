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
# both decide the same ties earlier, so this pair holds them off. A
# zero carried by a variable quenching an ordinary product (m430) takes
# the frames' identity rotations out of the cone and decides the tie
# for `b3.body.v_0[2]` before the weighing is asked, so it is held off
# the same way. The fold of continuous variables held still (m521)
# decides the tie before the weighing too, and the model then stops at
# an algebraic loop with reduction 86 never reached, so it is held off
# with the rest.
before=(OXIDELICA_NO_CONSTANT_CANDIDATES=1 OXIDELICA_NO_STATE_SELECT=1
  OXIDELICA_ZERO_VAR_DER_ONLY=1 OXIDELICA_NO_STILL_FOLD=1)
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
model="tests/small/a_delta_secondary_does_not_spend_its_angles_twice.mo"
[ -f "$model" ] || { echo "WITNESS: no model at $model"; exit 1; }
check "implied row refused" 8 "transformer.r2.resistor[2].pin_p.reference.gamma" \
  "$(victim_at 8 OXIDELICA_UNUSED=1)"
check "implied row taken" 8 "transformer.l2sigma.inductor[2].pin_p.reference.gamma" \
  "$(victim_at 8 OXIDELICA_IMPLIED_ROW=0 OXIDELICA_SUPPLY_ONCE=0)"
# With the chain on (the default since m410) the same member is refused
# a second time over: the dependence check of `=4`, not the ledger,
# reads its row as one the system already holds, which is the implied
# row seen from the linear side. So the implied row alone taken back
# changes nothing here, and the old victim needs both rules off.
check "implied row taken, ledger on" 8 "transformer.r2.resistor[2].pin_p.reference.gamma" \
  "$(victim_at 8 OXIDELICA_IMPLIED_ROW=0)"
# A row the system already holds, reached through equations that are all
# different: the star sum of a machine's load stated a second time. With
# the dependence of the supplied rows read (the default, `=5`), the
# reduction refuses the members that give it and demotes a stator
# current; with it taken back (`=0`), the load's inductor current goes
# and the block is singular.
model="tests/small/a_star_sum_stated_again_through_other_equations.mo"
[ -f "$model" ] || { echo "WITNESS: no model at $model"; exit 1; }
check "dependent row refused" 14 "smee.lssigma.i_[1]" "$(victim_at 14 OXIDELICA_SUPPLY_ONCE=4)"
check "dependent row refused, ledger first" 14 "smee.lssigma.i_[1]" \
  "$(victim_at 14 OXIDELICA_UNUSED=1)"
check "dependent row taken" 14 "loadInductor.inductor[1].i" "$(victim_at 14 OXIDELICA_SUPPLY_ONCE=0)"
exit "$status"
