connector Pin "Electrical pin"
  Real v "potential";
  flow Real i "current flowing into the component";
end Pin;

model TwoPin "Base for two-pin electrical components"
  Pin p "positive pin";
  Pin n "negative pin";
  Real v "voltage across, p minus n";
  Real i "current through, p to n";
equation
  v = p.v - n.v;
  p.i = i;
  n.i = -i;
end TwoPin;

model Resistor "Ideal resistor: v = R * i"
  extends TwoPin;
  parameter Real R = 1.0 "resistance";
equation
  i = v / R;
end Resistor;

model Capacitor "Ideal capacitor: C * der(v) = i"
  extends TwoPin;
  parameter Real C = 1.0 "capacitance";
equation
  der(v) = i / C;
end Capacitor;

model ConstantVoltage "Ideal constant voltage source"
  extends TwoPin;
  parameter Real V = 1.0 "source voltage";
equation
  v = V;
end ConstantVoltage;

model Ground "Reference potential"
  Pin p;
equation
  p.v = 0;
end Ground;

model RCCircuit "RC low-pass: capacitor charges to the source voltage"
  ConstantVoltage source(V = 1.0);
  Resistor r(R = 100.0);
  Capacitor c(C = 0.001);
  Ground ground;
equation
  connect(source.p, r.p);
  connect(r.n, c.p);
  connect(c.n, source.n);
  connect(c.n, ground.p);
  annotation(experiment(StopTime = 0.5, Interval = 0.0005, Tolerance = 1e-9));
end RCCircuit;
