model PendulumPeriod "a large-swing pendulum meets its exact period"
  // At large amplitude the pendulum period is an elliptic integral;
  // the arithmetic-geometric mean computes it in a handful of `while`
  // rounds - at compile time, since algorithms run symbolically.
  function exact_period "pendulum period via the arithmetic-geometric mean"
    input Real length;
    input Real gravity;
    input Real amplitude "swing amplitude in radians";
    output Real period;
  protected
    Real a;
    Real b;
    Real t;
  algorithm
    a := 1;
    b := cos(amplitude / 2);
    while abs(a - b) > 1e-15 loop
      t := (a + b) / 2;
      b := sqrt(a * b);
      a := t;
    end while;
    period := 4 * asin(1) * sqrt(length / gravity) / a;
  end exact_period;

  parameter Real L = 1.2 "length";
  parameter Real g = 9.81 "gravity";
  parameter Real amplitude = 1.0 "initial swing";
  Real period "the exact large-swing period";
  Real theta(start = amplitude) "swing angle";
  Real w(start = 0) "angular speed";
equation
  period = exact_period(L, g, amplitude);
  der(theta) = w;
  der(w) = -(g / L) * sin(theta);
  annotation(experiment(StopTime = 2.4, Interval = 0.001));
end PendulumPeriod;
