model CartesianPendulum "Index-3 DAE: a pendulum in Cartesian coordinates with a length constraint"
  parameter Real m = 1.0 "bob mass";
  parameter Real g = 9.81 "gravitational acceleration";
  parameter Real L = 1.0 "rod length";
  Real x(start = 0.6442176872376911) "bob x, starts at L*sin(0.7)";
  Real y(start = -0.7648421872844885) "bob y, starts at -L*cos(0.7)";
  Real vx(start = 0.0);
  Real vy(start = 0.0);
  Real lambda "rod tension per unit length (the constraint multiplier)";
equation
  der(x) = vx;
  der(y) = vy;
  der(vx) = -lambda * x / m;
  der(vy) = -lambda * y / m - g;
  x ^ 2 + y ^ 2 = L ^ 2;
  annotation(experiment(StopTime = 10.0, Interval = 0.001, Tolerance = 1e-10));
end CartesianPendulum;
