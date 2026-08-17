model Saturation "if-выражения: насыщение сигнала на уровнях ±1"
  Real x(start = -3.0, fixed = true) "линейно растущий вход";
  Real y "выход с насыщением";
equation
  der(x) = 1;
  y = if x > 1 then 1 elseif x < -1 then -1 else x;
  annotation(experiment(StopTime = 6.0, Interval = 0.01));
end Saturation;
