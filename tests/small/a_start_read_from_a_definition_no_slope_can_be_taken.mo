// A start read from an equation whose other side has no derivative.
//
// The reading of a start solves the equation for the name, and
// solving means taking a slope - which means differentiating. A
// density read from a water table is `d = waterBaseProp_pT(p, T,
// 0)[9]`, and nothing differentiates a medium call, so the read was
// refused and the density began at zero. A mass `m = V*d` then began
// at zero too, and a mass of zero asks the tables for a density no
// water has: the model refuses on a Newton direction that cannot
// reduce the residual.
//
// The slope was never needed. `d` stands alone on one side and the
// other side does not mention it, so what the equation says about
// `d` is the other side, read off by inspection.
//
// With the fault present this runs with `T` far from its start and
// `d` at zero; `div` stands here for the medium call, being a
// builtin with no derivative that a small model can carry.
model StartReadWithoutASlope
  Real T(start = 293.15);
  Real d;
  Real m;
equation
  d = 900 + div(T, 3);
  m = 1.0 * d;
  der(m) = 0;
end StartReadWithoutASlope;
