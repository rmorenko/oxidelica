// Squeezed down from
// Modelica.Magnetic.QuasiStatic.FundamentalWave.Examples.Components.EddyCurrentLosses.
//
// `TwoPortElementary` declares `omega = der(port_p.reference.gamma)`,
// and `EddyCurrent`, which extends it, declares the very same line
// again. A repeated declaration is one element and not two; taken as
// two, the model has two equations for one derivative and is refused
// as `two equations for der(g)`. Twenty-four models of the standard
// library stood at that wall.
model Base
  Real w = der(g);
  Real g;
end Base;

model ADeclarationRepeatedFromABase
  extends Base;
  Real w = der(g);
equation
  g = 2.0 * time;
  annotation(experiment(StopTime = 1));
end ADeclarationRepeatedFromABase;
