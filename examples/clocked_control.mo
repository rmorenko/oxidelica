model ClockedControl "a clocked PI controller on a continuous plant"
  // Chapter 16 in the small: the clock is declared, not implied, and
  // the equations that belong to it are written in ordinary form. The
  // compiler works out which ones those are and lifts them onto the
  // tick, so `previous` reaches back exactly one period and `hold`
  // brings the answer back to continuous time.
  //
  // Nothing here is approximate. Between ticks the control is a
  // constant, so the plant is a first-order lag relaxing towards it,
  // and the whole run is a recurrence that can be written out by hand.
  parameter Real Ts = 0.05 "controller period";
  parameter Real Tp = 0.4 "plant time constant";
  parameter Real kp = 1.6 "proportional gain";
  parameter Real ki = 4.0 "integral gain";
  parameter Real setpoint = 1;

  Clock c = Clock(0.05) "the controller's clock";

  Real x(start = 0, fixed = true) "what the plant does";
  Real error "setpoint minus the sampled measurement";
  Real integral "the running sum, one tick at a time";
  Real command "what the controller decides";
  Real u "the command, held between ticks";
equation
  // The clocked partition. `sample` reads the plant at the tick,
  // `interval` is the period the clock was declared with.
  error = setpoint - sample(x, c);
  integral = previous(integral) + error * interval(c);
  command = kp * error + ki * integral;

  // Back to continuous time, and the plant itself.
  u = hold(command);
  der(x) = (u - x) / Tp;
  annotation(experiment(StopTime = 3, Interval = 0.001, Tolerance = 1e-12));
end ClockedControl;
