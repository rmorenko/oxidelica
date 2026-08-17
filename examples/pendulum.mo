model Pendulum "Плоский маятник в угловых координатах + декартовы координаты груза"
  parameter Real g = 9.81 "ускорение свободного падения";
  parameter Real L = 1.0 "длина подвеса";
  Real phi(start = 0.7, fixed = true) "угол от вертикали";
  Real w(start = 0.0, fixed = true) "угловая скорость";
  Real x "декартова x груза";
  Real y "декартова y груза";
equation
  der(phi) = w;
  der(w) = -(g / L) * sin(phi);
  x = L * sin(phi);
  y = -L * cos(phi);
  annotation(experiment(StopTime = 10.0, Interval = 0.001));
end Pendulum;
