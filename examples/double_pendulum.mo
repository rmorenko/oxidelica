model DoublePendulum "Chaotic two-link pendulum, drawn as oriented bodies in 3D"
  import Oxidelica.Visualizers;
  import Oxidelica.Constants;

  parameter Real m1 = 1.0 "mass at the elbow";
  parameter Real m2 = 1.0 "mass at the tip";
  parameter Real L1 = 0.6 "length of link 1";
  parameter Real L2 = 0.5 "length of link 2";
  parameter Real g = 9.81;

  // Absolute angles from the downward vertical: the canonical form of
  // the point-mass double pendulum.
  Real th1(start = 1.6, fixed = true) "angle of link 1";
  Real th2(start = 1.2, fixed = true) "angle of link 2";
  Real w1(start = 0.0, fixed = true);
  Real w2(start = 0.0, fixed = true);
  Real a1 "angular acceleration of link 1";
  Real a2 "angular acceleration of link 2";
  Real x1 "elbow x";
  Real y1 "elbow y";
  Real x2 "tip x";
  Real y2 "tip y";
  Real energy "total mechanical energy, conserved";

  Visualizers.Shape link1(kind = 0, length = L1, width = 0.045, height = 0.045,
    red = 0.21, green = 0.45, blue = 0.94);
  Visualizers.Shape link2(kind = 0, length = L2, width = 0.045, height = 0.045,
    red = 0.94, green = 0.65, blue = 0.20);
  Visualizers.Shape elbow(kind = 1, length = 0.08, width = 0.08, height = 0.08,
    red = 0.34, green = 0.61, blue = 0.36);
  Visualizers.Shape tip(kind = 1, length = 0.11, width = 0.11, height = 0.11,
    red = 0.91, green = 0.29, blue = 0.64);
equation
  der(th1) = w1;
  der(th2) = w2;
  der(w1) = a1;
  der(w2) = a2;

  (m1 + m2) * L1 * a1 + m2 * L2 * a2 * cos(th1 - th2)
    + m2 * L2 * w2 ^ 2 * sin(th1 - th2) + (m1 + m2) * g * sin(th1) = 0;
  L2 * a2 + L1 * a1 * cos(th1 - th2) - L1 * w1 ^ 2 * sin(th1 - th2)
    + g * sin(th2) = 0;

  x1 = L1 * sin(th1);
  y1 = -L1 * cos(th1);
  x2 = x1 + L2 * sin(th2);
  y2 = y1 - L2 * cos(th2);

  energy = 0.5 * m1 * (L1 * w1) ^ 2
    + 0.5 * m2 * ((L1 * w1) ^ 2 + (L2 * w2) ^ 2
      + 2 * L1 * L2 * w1 * w2 * cos(th1 - th2))
    - (m1 + m2) * g * L1 * cos(th1) - m2 * g * L2 * cos(th2);

  // Each rod is drawn at its own midpoint, turned to its own angle; the
  // masses are spheres at the joints.
  link1.x = 0.5 * x1;
  link1.y = 0.5 * y1;
  link1.z = 0;
  // A shape's phi is measured from +x, the angles from the vertical.
  link1.phi = th1 - Constants.pi / 2;
  link2.x = 0.5 * (x1 + x2);
  link2.y = 0.5 * (y1 + y2);
  link2.z = 0;
  link2.phi = th2 - Constants.pi / 2;
  elbow.x = x1;
  elbow.y = y1;
  elbow.z = 0;
  elbow.phi = 0;
  tip.x = x2;
  tip.y = y2;
  tip.z = 0;
  tip.phi = 0;
  annotation(experiment(StopTime = 20.0, Interval = 0.005, Tolerance = 1e-10));
end DoublePendulum;
