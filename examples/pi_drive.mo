model ProportionalDrive "First-order plant with a replaceable controller"
  import Oxidelica.Blocks;

  parameter Real setpoint = 1.0 "commanded value";
  parameter Real gain = 5.0 "controller gain";

  Blocks.Sources.Step command(height = setpoint, startTime = 0.5);
  Blocks.Math.Feedback error;
  // Any block with one input and one output fits here; a derived model
  // swaps in a different one without touching the wiring below.
  replaceable Blocks.Math.Gain controller(k = gain) constrainedby Blocks.Interfaces.SISO;
  Blocks.Continuous.FirstOrder plant(k = 1.0, T = 0.5);

  Real y "plant output";
equation
  error.u1 = command.y;
  error.u2 = plant.y;
  controller.u = error.y;
  plant.u = controller.y;
  y = plant.y;
  annotation(experiment(StopTime = 6.0, Interval = 0.005, Tolerance = 1e-9));
end ProportionalDrive;

model PIDrive "The same drive with the controller redeclared as a PI"
  // Proportional control alone leaves a steady-state offset; the PI
  // removes it, and nothing but this line changes.
  extends ProportionalDrive(
    redeclare Oxidelica.Blocks.Continuous.PI controller(k = gain, T = 0.4));
  annotation(experiment(StopTime = 6.0, Interval = 0.005, Tolerance = 1e-9));
end PIDrive;
