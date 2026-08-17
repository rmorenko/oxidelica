function steadyState "Analytic steady-state temperature along the rod"
  input Real position "position as a fraction of the rod length";
  input Real left "temperature held at the left end";
  input Real right "temperature held at the right end";
  output Real value;
algorithm
  value := left + (right - left) * position;
end steadyState;

model HeatConduction "1D heat equation discretized into N nodes (method of lines)"
  parameter Integer N = 40 "number of interior nodes";
  parameter Real L = 1.0 "rod length";
  parameter Real alpha = 0.02 "thermal diffusivity";
  parameter Real Tleft = 100.0 "temperature held at the left end";
  parameter Real Tright = 20.0 "temperature held at the right end";
  parameter Real dx = L / (N + 1) "node spacing";
  Real T[N](start = 20.0) "node temperatures, rod starts cold";
  Real Tmid "temperature at the middle node";
  Real midError "deviation of the middle node from its steady state";
equation
  // Interior nodes: the discrete Laplacian of their neighbours.
  for i in 2:N - 1 loop
    der(T[i]) = alpha * (T[i - 1] - 2 * T[i] + T[i + 1]) / dx ^ 2;
  end for;
  // The end nodes see the held boundary temperatures.
  der(T[1]) = alpha * (Tleft - 2 * T[1] + T[2]) / dx ^ 2;
  der(T[N]) = alpha * (T[N - 1] - 2 * T[N] + Tright) / dx ^ 2;
  Tmid = T[20];
  midError = Tmid - steadyState(20 * dx / L, Tleft, Tright);
  annotation(experiment(StopTime = 60.0, Interval = 0.05, Tolerance = 1e-8));
end HeatConduction;
