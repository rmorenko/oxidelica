model Saturation "if-expressions: signal saturation at levels of plus/minus 1"
  Real x(start = -3.0, fixed = true) "linearly growing input";
  Real y "saturated output";
equation
  der(x) = 1;
  y = if x > 1 then 1 elseif x < -1 then -1 else x;
  annotation(experiment(StopTime = 6.0, Interval = 0.01));
end Saturation;
