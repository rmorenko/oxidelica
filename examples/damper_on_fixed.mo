model DamperOnFixed "A two-flange damper straight onto a fixed flange"
  // The relative angle of the damper is redundant with the shaft angle,
  // which makes this an index-2 system: reducing it means
  // differentiating a connection equality through connector potentials
  // that no equation defines explicitly. The compiler solves the linear
  // equations for them, so the chain of definitions grounds out and the
  // redundant state is demoted like any other.
  import Oxidelica.Mechanics.Rotational;
  Rotational.Components.Inertia shaft(J = 0.5, w_start = 5.0);
  Rotational.Components.Damper damper(d = 0.4);
  Rotational.Components.Fixed housing;
equation
  connect(shaft.flange_b, damper.flange_a);
  connect(damper.flange_b, housing.flange);
  annotation(experiment(StopTime = 5.0, Interval = 0.01, Tolerance = 1e-9));
end DamperOnFixed;
