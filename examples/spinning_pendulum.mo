model SpinningPendulum "A Cartesian pendulum going over the top - the case that needs dynamic state selection"
  // The length constraint can define y from x only while |y| is large;
  // as the bob approaches the horizontal the roles must swap. The
  // solver watches the sensitivity of each reduced constraint to its
  // demoted state and re-selects the states mid-run whenever the pivot
  // that chose them would now choose differently.
  parameter Real m = 1.0;
  parameter Real g = 9.81;
  parameter Real L = 1.0;
  Real x(start = 0.0) "bob x, starts at the bottom";
  Real y(start = -1.0) "bob y";
  Real vx(start = 8.0) "fast enough to keep rotating";
  Real vy(start = 0.0);
  Real lambda "rod tension per unit length";
equation
  der(x) = vx;
  der(y) = vy;
  der(vx) = -lambda * x / m;
  der(vy) = -lambda * y / m - g;
  x ^ 2 + y ^ 2 = L ^ 2;
  annotation(experiment(StopTime = 3.0, Interval = 0.002, Tolerance = 1e-9));
end SpinningPendulum;
