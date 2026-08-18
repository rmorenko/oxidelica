model MassChains "two chains of different length, one pair of functions"
  // The point of `Real v[:]`: these functions are written once and
  // measure whatever they are handed. The three-mass chain and the
  // five-mass chain below call exactly the same code, and the result of
  // `weighted` takes its length from its argument.
  //
  // Nothing pushes on either chain from outside, so each one's momentum
  // is constant and its centre of mass travels in a straight line -
  // which is what the run is checked against.
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

  parameter Real k = 4 "spring constant";
  parameter Real rest = 1 "unstretched spring length";

  parameter Real m3[3] = {1, 2, 3};
  Real x3[3](start = {0, 1, 2});
  Real v3[3](start = {1, 0, -0.5});
  Real f3[3];
  Real momentum3;
  Real centre3;

  parameter Real m5[5] = {1, 1, 2, 1, 1};
  Real x5[5](start = {0, 1, 2, 3, 4});
  Real v5[5](start = {0.5, 0, 0, 0, -0.5});
  Real f5[5];
  Real momentum5;
  Real centre5;
equation
  // Each interior spring pulls both of its neighbours; the ends have
  // only one spring, so neither chain is pushed from outside.
  for i in 1:3 loop
    f3[i] =
      (if i < 3 then k * (x3[i + 1] - x3[i] - rest) else 0)
      - (if i > 1 then k * (x3[i] - x3[i - 1] - rest) else 0);
    der(x3[i]) = v3[i];
    der(v3[i]) = f3[i] / m3[i];
  end for;
  momentum3 = total(weighted(m3, v3));
  centre3 = total(weighted(m3, x3)) / total(m3);

  for i in 1:5 loop
    f5[i] =
      (if i < 5 then k * (x5[i + 1] - x5[i] - rest) else 0)
      - (if i > 1 then k * (x5[i] - x5[i - 1] - rest) else 0);
    der(x5[i]) = v5[i];
    der(v5[i]) = f5[i] / m5[i];
  end for;
  momentum5 = total(weighted(m5, v5));
  centre5 = total(weighted(m5, x5)) / total(m5);
  annotation(experiment(StopTime = 4, Interval = 0.01, Tolerance = 1e-10));
end MassChains;
