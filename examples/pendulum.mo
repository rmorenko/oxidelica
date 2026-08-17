model Pendulum "Planar pendulum in angle coordinates plus Cartesian bob coordinates"
  parameter Real g = 9.81 "gravitational acceleration";
  parameter Real L = 1.0 "rod length";
  Real phi(start = 0.7, fixed = true) "angle from vertical";
  Real w(start = 0.0, fixed = true) "angular velocity";
  Real x "bob x coordinate";
  Real y "bob y coordinate";
equation
  der(phi) = w;
  der(w) = -(g / L) * sin(phi);
  x = L * sin(phi);
  y = -L * cos(phi);
  annotation(experiment(StopTime = 10.0, Interval = 0.001));
end Pendulum;
