model Decay "Экспоненциальный распад: der(x) = -a*x, аналитика x(t) = e^(-a*t)"
  parameter Real a = 1.0 "скорость распада";
  Real x(start = 1.0, fixed = true) "состояние";
equation
  der(x) = -a * x;
  annotation(experiment(StopTime = 5.0, Interval = 0.001));
end Decay;
