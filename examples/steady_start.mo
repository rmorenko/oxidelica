model SteadyStart "Two systems started in equilibrium instead of at rest"
  // Without the section at the bottom both would begin at their declared
  // start values and spend the run creeping toward the balance point;
  // `initial equation` asks the compiler to start there instead.
  parameter Real Toutside = 5.0 "outside temperature";
  parameter Real G = 250.0 "heat loss, W/K";
  parameter Real C = 5.0e4 "thermal capacity, J/K";
  parameter Real heater = 3.0e3 "heater power, W";

  parameter Real k = 40.0 "spring constant, N/m";
  parameter Real m = 2.0 "hanging mass, kg";
  parameter Real d = 6.0 "damping, N.s/m";
  parameter Real g = 9.81;

  Real T(start = 15.0) "room temperature";
  Real x(start = 0.0) "position of the mass, positive upwards";
  Real v(start = 0.0) "velocity of the mass";
equation
  der(T) = (heater - G * (T - Toutside)) / C;
  der(x) = v;
  der(v) = (-k * x - d * v) / m - g;
initial equation
  der(T) = 0;
  der(x) = 0;
  der(v) = 0;
  annotation(experiment(StopTime = 5.0, Interval = 0.01, Tolerance = 1e-9));
end SteadyStart;
