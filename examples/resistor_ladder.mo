model ResistorLadder "A divider chain: an array of resistors wired by a loop"
  import Oxidelica.Electrical.Analog;

  parameter Integer n = 5 "resistors in the chain";
  parameter Real V = 10.0 "supply voltage";

  Analog.Sources.ConstantVoltage supply(V = V);
  // One declaration for the whole chain; `each` gives every element the
  // same resistance.
  Analog.Basic.Resistor r[n](each R = 220);
  Analog.Basic.Ground ground;

  Real taps[n] "voltage after each resistor - the divider outputs";
equation
  connect(supply.p, r[1].p);
  // The wiring is a loop over the array, not n copied lines.
  for i in 1:n - 1 loop
    connect(r[i].n, r[i + 1].p);
  end for;
  connect(r[n].n, supply.n);
  connect(supply.n, ground.p);
  for i in 1:n loop
    taps[i] = r[i].n.v;
  end for;
  annotation(experiment(StopTime = 1.0, Interval = 0.5));
end ResistorLadder;
