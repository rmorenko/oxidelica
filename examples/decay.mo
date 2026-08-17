model Decay "Exponential decay: der(x) = -a*x, analytic x(t) = e^(-a*t)"
  parameter Real a = 1.0 "decay rate";
  Real x(start = 1.0, fixed = true) "state";
equation
  der(x) = -a * x;
  annotation(experiment(StopTime = 5.0, Interval = 0.001));
end Decay;
