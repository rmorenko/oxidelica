model BouncingBall "Events with reinit: a ball losing energy at every impact"
  parameter Real e = 0.8 "coefficient of restitution";
  parameter Real g = 9.81 "gravitational acceleration";
  parameter Real vRest = 0.3 "impact speed below which the ball is at rest";
  Real h(start = 1.0) "height above the floor";
  Real v(start = 0.0) "vertical velocity";
equation
  der(h) = v;
  der(v) = -g;
  when h < 0 then
    reinit(v, -e * v);
  end when;
  // Impacts crowd together as the ball settles (the Zeno limit), so the
  // idealized model is stopped once the bounces become negligible.
  when h < 0 and v > -vRest then
    terminate("the ball has come to rest");
  end when;
  annotation(experiment(StopTime = 10.0, Interval = 0.002, Tolerance = 1e-9));
end BouncingBall;
