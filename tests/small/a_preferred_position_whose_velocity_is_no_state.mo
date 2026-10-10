// Squeezed from Modelica.Magnetic.FluxTubes.Examples.MovingCoilActuator.
// ArmatureStroke to one actuator and its load, and standing for a
// preference this compiler can only keep half of.
//
// The armature's two ElastoGap stoppers declare `s_rel` and `v_rel`
// preferred, and `v_rel = der(s_rel)` makes the velocity the right-hand
// side of the position's state equation rather than a state of its own.
// Read for the position alone, the first reduction keeps
// `stopper_xMax.s_rel` and demotes the armature's `mass.s`; the
// velocity level then has nothing preferred to keep, the last default
// velocity goes, and the run stops at `the equations of algebraic loop
// [...] do not mention ["cActuator.armature.stopper_xMin.v_rel", ...]`.
//
// Read for the pair or not at all, the position weighs as the default,
// the first reduction demotes `stopper_xMax.s_rel` and the model runs
// with the file it gave before `stateSelect` was read.
// OXIDELICA_NO_HALF_PAIRS gives back the half reading, and
// `scripts/victim_witness.sh` asks both of one binary.
model a_preferred_position_whose_velocity_is_no_state
  import Modelica.Magnetic.FluxTubes;
  Modelica.Electrical.Analog.Sources.StepVoltage cSource(startTime=0, V=cActuator.R*1.5);
  Modelica.Electrical.Analog.Basic.Ground cGround;
  FluxTubes.Examples.MovingCoilActuator.Components.ConstantActuator cActuator(
    x(start=cActuator.x_min, fixed=true),
    armature(v(fixed=true)),
    l(i(start=0, fixed=true)));
  Modelica.Mechanics.Translational.Components.Mass cLoad(m=0.05);
equation
  connect(cLoad.flange_a, cActuator.flange);
  connect(cGround.p, cSource.n);
  connect(cSource.p, cActuator.p);
  connect(cActuator.n, cGround.p);
  annotation(experiment(StopTime=0.05));
end a_preferred_position_whose_velocity_is_no_state;
