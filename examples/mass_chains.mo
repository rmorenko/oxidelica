model MassChains "two chains of different length from one component"
  // Two things meet here. The functions are written once with `[:]`
  // inputs and measure whatever they are handed, and the chain itself
  // is written once with a parameter for its length: each instance
  // says how many masses it has and hands them over as a whole array,
  // which is spread to the elements.
  //
  // Nothing pushes on either chain from outside, so each one's
  // momentum is constant and its centre of mass travels in a straight
  // line - which is what the run is checked against.
  function total "sum of a vector of any length"
    input Real v[:];
    output Real s;
  algorithm
    s := 0;
    for i in 1:size(v, 1) loop
      s := s + v[i];
    end for;
  end total;

  function weighted "elementwise product, as long as its arguments"
    input Real w[:];
    input Real q[:];
    output Real p[size(w, 1)];
  algorithm
    for i in 1:size(w, 1) loop
      p[i] := w[i] * q[i];
    end for;
  end weighted;

  model Chain "masses in a row on springs, free at both ends"
    parameter Integer n = 3 "how many masses";
    parameter Real m[n] "the masses themselves";
    parameter Real k = 4 "spring constant";
    parameter Real rest = 1 "unstretched spring length";
    Real x[n] "positions";
    Real v[n] "velocities";
    Real force[n] "net force on each mass";
    Real momentum "the whole chain's momentum";
    Real centre "centre of mass";
  equation
    // Each interior spring pulls both of its neighbours; the ends have
    // only one, so nothing acts on the chain from outside.
    for i in 1:n loop
      force[i] =
        (if i < n then k * (x[i + 1] - x[i] - rest) else 0)
        - (if i > 1 then k * (x[i] - x[i - 1] - rest) else 0);
      der(x[i]) = v[i];
      der(v[i]) = force[i] / m[i];
    end for;
    momentum = total(weighted(m, v));
    centre = total(weighted(m, x)) / total(m);
  end Chain;

  Chain short(
    n = 3,
    m = {1, 2, 3},
    x(start = {0, 1, 2}),
    v(start = {1, 0, -0.5}));
  Chain long(
    n = 5,
    m = {1, 1, 2, 1, 1},
    x(start = {0, 1, 2, 3, 4}),
    v(start = {0.5, 0, 0, 0, -0.5}));
  annotation(experiment(StopTime = 4, Interval = 0.01, Tolerance = 1e-10));
end MassChains;
