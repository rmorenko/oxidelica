model DCMotor "DC motor driving an inertia: electrical, mechanical and block domains combined"
  import Oxidelica.Electrical.Analog;
  import Oxidelica.Mechanics.Rotational;

  parameter Real V = 24.0 "supply voltage";
  parameter Real R = 0.5 "armature resistance";
  parameter Real L = 0.05 "armature inductance";
  parameter Real k = 0.3 "motor constant";
  parameter Real J = 0.02 "load inertia";
  parameter Real d = 0.02 "viscous friction";

  Analog.Sources.StepVoltage supply(V = V, startTime = 0.1);
  Analog.Basic.Resistor armature(R = R);
  Analog.Basic.Inductor winding(L = L);
  Analog.Basic.EMF emf(k = k);
  Analog.Basic.Ground ground;
  Rotational.Components.Inertia load(J = J);
  Rotational.Components.ViscousFriction friction(d = d);
  Real speed "shaft speed in rad/s";
  Real current "armature current in A";
equation
  connect(supply.p, armature.p);
  connect(armature.n, winding.p);
  connect(winding.n, emf.p);
  connect(emf.n, supply.n);
  connect(supply.n, ground.p);
  connect(emf.flange, load.flange_a);
  connect(load.flange_b, friction.flange);
  speed = load.w;
  current = armature.i;
  annotation(experiment(StopTime = 2.0, Interval = 0.002, Tolerance = 1e-9));
end DCMotor;
