model TransportDelay "a heated pipe, and the wave that walks down it"
  // `delay(u, T)` is what `u` was T ago. Here it is a pipe: whatever
  // the heater does to the fluid at the inlet arrives at the outlet a
  // transit time later, unchanged but for the loss along the way.
  //
  // Nothing is approximated in the shape of the wave - it is the
  // inlet's own, shifted. What the run keeps is the inlet at each
  // output point, so the shift is only as exact as the output is fine;
  // halving `Interval` quarters the error, which is what a straight
  // line between two remembered points does.
  parameter Real transit = 0.53 "how long the fluid takes to cross";
  parameter Real loss = 0.15 "fraction lost on the way";
  parameter Real tank = 0.4 "time constant of the vessel it pours into";

  Real inlet "temperature the heater leaves behind";
  Real outlet "what arrives at the far end";
  Real vessel(start = 0, fixed = true) "the vessel it pours into";
equation
  inlet = sin(3 * time);
  outlet = (1 - loss) * delay(inlet, transit);
  der(vessel) = (outlet - vessel) / tank;
  annotation(experiment(StopTime = 6, Interval = 0.001, Tolerance = 1e-10));
end TransportDelay;
