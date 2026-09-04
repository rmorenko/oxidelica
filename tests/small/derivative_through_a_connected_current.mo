// Squeezed down from
// Modelica.Magnetic.FluxTubes.Examples.MovingCoilActuator.ForceCurrentBehaviour
// by dropping the half of the model the refusal does not name.
//
// Index reduction has to differentiate through `pmActuator.r.p.i`,
// which no equation defines outright: it is pinned by connection
// equations - `0 = r.p.i + r.n.i`, `r.i = r.p.i`, `-p.i + r.p.i = 0`.
// Definitions are accepted only once everything they mention is
// itself grounded, and these mention each other, so none of them is
// taken and the model is refused as structurally singular.
//
// `pmActuator.p.i` *is* in the table, and `r.p.i` is equal to it by
// one of those equations, so the chain is one step from closing.
model DerivativeThroughAConnectedCurrent
  Modelica.Magnetic.FluxTubes.Examples.MovingCoilActuator.Components.PermeanceActuator
    pmActuator(x(start = 0));
  Modelica.Mechanics.Translational.Components.Fixed pmFixedPos(s0 = 0);
  Modelica.Electrical.Analog.Sources.RampCurrent pmRampCurrent(I = -6, duration = 6, offset = 3);
  Modelica.Electrical.Analog.Basic.Ground pmGround;
equation
  connect(pmActuator.flange, pmFixedPos.flange);
  connect(pmRampCurrent.p, pmActuator.p);
  connect(pmActuator.n, pmRampCurrent.n);
  connect(pmRampCurrent.n, pmGround.p);
  annotation(experiment(StopTime = 6));
end DerivativeThroughAConnectedCurrent;
