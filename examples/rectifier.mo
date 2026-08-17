model Rectifier "Ideal diode: a half-wave rectifier with an RC load"
  parameter Real A = 1.0 "source amplitude";
  parameter Real w = 6.283185307179586 "angular frequency (1 Hz)";
  parameter Real R = 0.5 "series resistance";
  parameter Real RL = 20.0 "load resistance";
  parameter Real C = 0.1 "load capacitance";
  Real vs "source voltage";
  Real vc(start = 0.0) "load voltage";
  Real id "diode current: zero unless forward biased";
equation
  vs = A * sin(w * time);
  // An ideal diode conducts only while forward biased; the branch
  // switch is an event, located exactly rather than stepped over.
  id = if vs - vc > 0 then (vs - vc) / R else 0;
  der(vc) = (id - vc / RL) / C;
  annotation(experiment(StopTime = 4.0, Interval = 0.001, Tolerance = 1e-9));
end Rectifier;
