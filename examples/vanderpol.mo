model VanDerPol "Van der Pol oscillator: a limit cycle from any start"
  parameter Real mu = 1.0 "nonlinear damping";
  Real x(start = 0.1, fixed = true);
  Real v(start = 0.0, fixed = true);
equation
  der(x) = v;
  der(v) = mu * (1 - x ^ 2) * v - x;
  annotation(experiment(StopTime = 20.0, Interval = 0.001));
end VanDerPol;
