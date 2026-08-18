model TrafficLight "a state machine holding up a queue of cars"
  // Chapter 17: the states are ordinary blocks, the arrows between
  // them are declared, and the machine runs on a clock. A state's
  // equations count only while it is the state the machine is in; the
  // others hold what they had, and a state entered afresh starts over.
  //
  // The queue underneath is continuous and knows nothing about any of
  // it: cars arrive at a steady rate and leave only while the light is
  // green, so the queue is a sawtooth whose corners can be counted.
  block Phase "one colour, counting how long it has been shown"
    parameter Real shown_for = 3 "ticks to hold this colour";
    Real elapsed(start = 0);
  equation
    elapsed = previous(elapsed) + 1;
  end Phase;

  parameter Real arrivals = 2 "cars joining the queue each second";
  parameter Real departures = 5 "cars leaving each second, while green";

  Clock tick = Clock(1.0) "the light thinks once a second";
  Phase red(shown_for = 3);
  Phase green(shown_for = 4);
  Phase amber(shown_for = 1);

  Real lamp "0 red, 1 green, 2 amber";
  Real flowing "how fast the queue drains just now";
  Real queue(start = 0, fixed = true) "cars waiting";
equation
  initialState(red);
  transition(red, green, red.elapsed >= red.shown_for);
  transition(green, amber, green.elapsed >= green.shown_for);
  transition(amber, red, amber.elapsed >= amber.shown_for);

  lamp = if activeState(red) then 0 elseif activeState(green) then 1 else 2;
  flowing = if activeState(green) then departures else 0;

  // Back in continuous time, where the cars are.
  der(queue) = arrivals - hold(flowing);
  annotation(experiment(StopTime = 22, Interval = 0.01, Tolerance = 1e-12));
end TrafficLight;
