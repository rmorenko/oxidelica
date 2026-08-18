model BallisticRange "A thrown ball checked against its own flight plan"
  // One call fills both targets: the function computes the closed-form
  // landing point and flight time, and the simulated trajectory should
  // arrive exactly there. The third input takes its default.
  function flight "closed-form range and duration of a throw"
    input Real v0 "launch speed";
    input Real angle "launch angle";
    input Real g = 9.81 "gravity, defaulted";
    output Real range "where the ball lands";
    output Real duration "when it lands";
  algorithm
    range := v0 ^ 2 * sin(2 * angle) / g;
    duration := 2 * v0 * sin(angle) / g;
  end flight;

  parameter Real v0 = 12 "launch speed";
  parameter Real angle = 0.6 "launch angle";
  parameter Real g = 9.81 "gravity";
  Real planned_range "the flight plan's landing point";
  Real planned_duration "the flight plan's landing time";
  Real x(start = 0) "horizontal position";
  Real y(start = 0) "height";
  Real vx(start = v0 * cos(angle));
  Real vy(start = v0 * sin(angle));
equation
  (planned_range, planned_duration) = flight(v0, angle);
  der(x) = vx;
  der(y) = vy;
  der(vx) = 0;
  der(vy) = -g;
  annotation(experiment(StopTime = 1.3814, Interval = 0.01));
end BallisticRange;
