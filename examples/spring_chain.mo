model SpringChain "A chain of masses on springs, described with arrays rather than one variable per body"
  parameter Integer n = 5 "number of masses";
  parameter Real m[n] = {1.0, 1.4, 0.8, 1.2, 1.0} "mass of each body, kg";
  parameter Real k[n + 1] = fill(60.0, n + 1) "stiffness of every spring, N/m";
  parameter Real span = 3.0 "distance between the two walls, m";
  parameter Real rest = span / (n + 1) "rest length of each spring, m";

  // Where the bodies would sit at rest, and where they start: the whole
  // chain is described in one line each instead of one per body.
  parameter Real home[n] = linspace(rest, span - rest, n) "resting positions";
  parameter Real push = 2.0 "initial speed given to the first body, m/s";
  Real x[n](start = home) "positions along the chain, m";
  Real v[n](start = {push, 0, 0, 0, 0}) "velocities, m/s";

  Real stretch[n + 1] "extension of each spring, m";
  Real kinetic "kinetic energy, J";
  Real potential "energy stored in the springs, J";
  Real energy "total mechanical energy: conserved, since nothing damps";
equation
  // The springs, counted from the left wall to the right one.
  stretch[1] = x[1] - rest;
  for i in 2:n loop
    stretch[i] = x[i] - x[i - 1] - rest;
  end for;
  stretch[n + 1] = span - x[n] - rest;

  // Newton for every body, written with the arrays as wholes where the
  // shape allows it.
  der(x) = v;
  for i in 1:n loop
    der(v[i]) = (k[i + 1] * stretch[i + 1] - k[i] * stretch[i]) / m[i];
  end for;

  kinetic = 0.5 * sum(m .* v .* v);
  potential = 0.5 * sum(k .* stretch .* stretch);
  energy = kinetic + potential;
  annotation(experiment(StopTime = 4.0, Interval = 0.002, Tolerance = 1e-9));
end SpringChain;
