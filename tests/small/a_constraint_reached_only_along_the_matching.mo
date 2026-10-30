// Squeezed from Modelica.Electrical.Machines.Examples.
// SynchronousMachines.SMPM_NoLoad to the machine, its terminal box on
// an open star and a speed source. Its thirteenth reduction is a
// Kirchhoff row of the star whose every member, read through the table
// of definitions, constrains no state: the earlier reductions left the
// air gap's rotation with two unsettled names a row. Along the
// matching's alternating path the same row reaches the rotor flux, and
// the reduction demotes `smpm.airGap.psi_mr[2]` and then
// `smpm.inertiaRotor.w` (docs/ANALYSIS.md, m542).
model a_constraint_reached_only_along_the_matching
  import Modelica.Constants.pi;
  parameter Modelica.Electrical.Machines.Utilities.ParameterRecords.SM_PermanentMagnetData smpmData(useDamperCage=false);
  Modelica.Electrical.Machines.BasicMachines.SynchronousMachines.SM_PermanentMagnet smpm(
    p=smpmData.p, fsNominal=smpmData.fsNominal, Rs=smpmData.Rs, TsRef=smpmData.TsRef,
    Lszero=smpmData.Lszero, Lssigma=smpmData.Lssigma, Jr=smpmData.Jr,
    VsOpenCircuit=smpmData.VsOpenCircuit, Lmd=smpmData.Lmd, Lmq=smpmData.Lmq,
    useDamperCage=false, TsOperational=293.15, TrOperational=293.15,
    phiMechanical(fixed=true, start=pi/smpmData.p));
  Modelica.Electrical.Machines.Utilities.TerminalBox terminalBox(terminalConnection="Y", m=3);
  Modelica.Electrical.Analog.Basic.Ground ground;
  Modelica.Mechanics.Rotational.Sources.Speed speed(exact=true, phi(fixed=false));
  Modelica.Blocks.Sources.Constant c(k=100);
equation
  connect(terminalBox.plug_sn, smpm.plug_sn);
  connect(terminalBox.plug_sp, smpm.plug_sp);
  connect(ground.p, terminalBox.starpoint);
  connect(c.y, speed.w_ref);
  connect(speed.flange, smpm.flange);
end a_constraint_reached_only_along_the_matching;
