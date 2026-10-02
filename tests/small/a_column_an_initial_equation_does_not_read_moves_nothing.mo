// A column of the initialisation's difference Jacobian is moved by
// the unknown it perturbs and by nothing else. Cut down from
// BranchingPipes17: a pipe, a valve and a tee volume between two
// boundaries. The tee's medium temperature is an inner block solved
// again at every column, and solved from wherever the previous column
// left it, it settled a rounding's width away each time - which over
// a step of 2e-7 read as a slope in columns its row never reads. The
// step that followed was wrong, and the run refused with "the
// equations of algebraic loop [..] do not mention
// [der(junctionVolume.medium.h)]" (in the whole model, a mass of
// -1560 kg in a litre). Each column now starts the inner blocks from
// the point the residual was taken at; OXIDELICA_INIT_DRIFT_GUESS
// brings the old behaviour back.
model a_column_an_initial_equation_does_not_read_moves_nothing
  package Medium = Modelica.Media.Air.DryAirNasa;
  inner Modelica.Fluid.System system(energyDynamics=Modelica.Fluid.Types.Dynamics.FixedInitial, use_eps_Re=true);
  Modelica.Fluid.Sources.Boundary_pT source(nPorts=1, redeclare package Medium = Medium, p=5.0e5, T=300);
  Modelica.Fluid.Sources.Boundary_pT sink(nPorts=1, redeclare package Medium = Medium, T=300, p=1.0e5);
  Modelica.Fluid.Pipes.DynamicPipe pipe3(redeclare package Medium = Medium, use_T_start=true,
    redeclare model FlowModel = Modelica.Fluid.Pipes.BaseClasses.FlowModels.TurbulentPipeFlow,
    length=10, diameter=2.54e-2, p_a_start=495000, p_b_start=490000, nNodes=1,
    modelStructure=Modelica.Fluid.Types.ModelStructure.a_vb);
  Modelica.Fluid.Valves.ValveCompressible valve2(redeclare package Medium = Medium,
    CvData=Modelica.Fluid.Types.CvTypes.OpPoint, m_flow_nominal=1, rho_nominal=5,
    dp_nominal=100000, p_nominal=400000, opening=1);
  Modelica.Fluid.Fittings.TeeJunctionVolume junctionVolume(redeclare package Medium = Medium, V=1e-3,
    massDynamics=Modelica.Fluid.Types.Dynamics.DynamicFreeInitial);
equation
  connect(source.ports[1], pipe3.port_a);
  connect(pipe3.port_b, valve2.port_a);
  connect(junctionVolume.port_3, sink.ports[1]);
  connect(valve2.port_b, junctionVolume.port_1);
  annotation(experiment(StopTime=0.01));
end a_column_an_initial_equation_does_not_read_moves_nothing;
