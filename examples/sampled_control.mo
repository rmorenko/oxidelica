model SampledControl "A digital PI controller holding its output between samples"
  parameter Real Ts = 0.1 "sampling period";
  parameter Real setpoint = 1.0 "commanded value";
  parameter Real K = 2.0 "controller gain";
  parameter Real Ti = 0.5 "integral time";
  parameter Real T = 0.5 "plant time constant";

  Real y(start = 0.0, fixed = true) "plant output";
  // The controller runs on a clock: both variables keep their value
  // between samples, so the plant sees a staircase, not a curve.
  Real u "control signal, held between samples";
  Real integral "integrator state of the controller";
  Real e "control error at the last sample";
equation
  der(y) = (u - y) / T;
  when sample(0, Ts) then
    e = setpoint - y;
    integral = pre(integral) + Ts * e / Ti;
    u = K * (e + integral);
  end when;
  annotation(experiment(StopTime = 5.0, Interval = 0.002, Tolerance = 1e-9));
end SampledControl;
