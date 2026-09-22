// Squeezed down from the six Fluid and Media models whose initial
// equation anchors `der(volume.medium.T)` or `der(pump.medium.h)`.
//
// `y` is determined by an equation no rearrangement solves for it,
// so the plan tears it and Newton iterates. The initialisation held
// that against it: `der(y)` was refused as `y` not being a state,
// although the residual determining `y` is exactly the shape the
// implicit function theorem wants. With the theorem read, `der(y)=0`
// forces `x = a = 3` and `y^3 + y = 3`, so `y = 1.2134116627622316`.
//
// Without the fix: `der(y): `y` is not a state of the model`.
model ATornUnknownHasADerivativeByTheTheorem
  parameter Real a = 3.0;
  Real x(start = 1.0);
  Real y(start = 0.5);
initial equation
  der(y) = 0;
equation
  y ^ 3 + y = x;
  der(x) = a - x;
  annotation(experiment(StopTime = 1));
end ATornUnknownHasADerivativeByTheTheorem;
