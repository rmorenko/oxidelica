model TorqueSupport "The same drive twice: an internal housing and an exposed support"
  import Oxidelica.Mechanics.Rotational;

  parameter Oxidelica.Units.Torque tau = 2.0 "driving torque";
  parameter Oxidelica.Units.Inertia J = 0.5 "inertia of each shaft";

  // Left drive: `useSupport = false`, so the support flange does not
  // exist and the torque reacts on the ground inside the source.
  Rotational.Sources.Torque driveA(tau_constant = tau, useSupport = false);
  Rotational.Components.Inertia shaftA(J = J);

  // Right drive: the support flange exists and is bolted to a housing
  // in this model instead.
  Rotational.Sources.Torque driveB(tau_constant = tau, useSupport = true);
  Rotational.Components.Inertia shaftB(J = J);
  Rotational.Components.Fixed housing;

  Real difference "shaftA.phi - shaftB.phi: zero, whichever support is used";
equation
  connect(driveA.flange, shaftA.flange_a);
  connect(driveB.flange, shaftB.flange_a);
  connect(driveB.support, housing.flange);
  difference = shaftA.phi - shaftB.phi;
  annotation(experiment(StopTime = 4.0, Interval = 0.01, Tolerance = 1e-9));
end TorqueSupport;
