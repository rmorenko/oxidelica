// A magnetic permeance is never written down directly: the library
// writes the reluctance it makes, `R_m = 1/G_m`, and leaves `G_m` to
// be worked back out. Linear solving cannot reach it - the equation
// is linear in the reciprocal and in nothing else - so the equation
// used to join the tearing set, where Newton was handed a block of
// one whose derivative is `-1/G_m^2`. That slope is enormous beside
// the pole and flat away from it, so an iteration started off the
// zero walks outward until the slope dies away faster than the
// residual does, and the refusal that came back was
// `singular Jacobian in algebraic loop ["G_m"]` - about an equation
// with one plain answer.
//
// Solved for the reciprocal and inverted, there is no iteration at
// all. With `Rtot = 4` and `c = 0.8`, the leakage pair fixes
// `R_m = c*Rtot/(1 - c) = 16`, so `G_m = 0.0625` and, with `V_m = 10`,
// `Phi = 0.625`.

model an_unknown_that_stands_only_under_a_division
  parameter Real c = 0.8 "ratio of useful flux to total flux";
  parameter Real Rtot = 4 "reluctance of the useful flux path";
  Real R_m "reluctance of the leakage path";
  Real G_m "permeance of the leakage path";
  Real Phi "flux through it";
  Real V_m "magnetic potential difference across it";
equation
  V_m = 10;
  V_m = Phi*R_m;
  R_m = 1/G_m;
  (1 - c)*R_m = c*Rtot;
end an_unknown_that_stands_only_under_a_division;
