model Thermostat "A room heated by a thermostat with a hysteresis band"
  parameter Real Tset = 20.0 "target temperature";
  parameter Real band = 1.0 "the heater switches at Tset plus and minus this";
  parameter Real C = 5.0e4 "thermal capacity of the room, J/K";
  parameter Real G = 250.0 "heat loss to the outside, W/K";
  parameter Real Toutside = 5.0 "outside temperature";
  parameter Real power = 6.0e3 "heater power, W";

  Real T(start = 15.0, fixed = true) "room temperature";
  // Both variables change only at events, so they are discrete: the
  // heater state remembers itself between switches and the counter
  // reads its own previous value.
  Boolean heating(start = true) "true while the heater runs";
  Integer switches(start = 0) "how often the heater has turned on";
equation
  der(T) = ((if heating then power else 0.0) - G * (T - Toutside)) / C;
  when T > Tset + band then
    heating = false;
    switches = pre(switches);
  elsewhen T < Tset - band then
    heating = true;
    switches = pre(switches) + 1;
  end when;
  annotation(experiment(StopTime = 7200.0, Interval = 2.0, Tolerance = 1e-9));
end Thermostat;
