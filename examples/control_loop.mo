model ControlLoop "PI control of a first-order plant, built from library blocks"
  import Oxidelica.Blocks;

  parameter Real setpoint = 1.0 "commanded value";
  parameter Real kp = 2.0 "controller gain";
  parameter Real Ti = 0.5 "controller integral time";
  parameter Real plantGain = 1.0;
  parameter Real plantTime = 1.0;

  Blocks.Sources.Step command(height = setpoint, startTime = 0.5);
  Blocks.Math.Feedback error;
  Blocks.Continuous.PI controller(k = kp, T = Ti);
  Blocks.Continuous.FirstOrder plant(k = plantGain, T = plantTime);
  Real y "plant output";
  Real e "control error";
equation
  error.u1 = command.y;
  error.u2 = plant.y;
  controller.u = error.y;
  plant.u = controller.y;
  y = plant.y;
  e = error.y;
  annotation(experiment(StopTime = 10.0, Interval = 0.005, Tolerance = 1e-9));
end ControlLoop;
