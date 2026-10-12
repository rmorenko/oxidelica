// Squeezed from Modelica.Electrical.Machines.Examples.
// SynchronousMachines.SMEE_LoadDump to the machine, its voltage
// controller and a star-connected RL load, standing for a reduction
// that adds a row the system already holds without differentiating any
// one equation twice.
//
// At reduction 14 the matching hands over members of a singular subset
// of 32 whose differentiated rows, followed through the stray-load and
// terminal currents, add up to the star point's current balance a
// second time: `der(smee.rs.plug_p.pin[1].i) +
// loadInductor.inductor[1].v / loadInductor.inductor[1].L = 0` is
// `-der(voltageQuasiRMSSensor.plug_p.pin[1].i) = 0` once the block's
// inner assignments are read. No equation index repeats, so a ledger of
// indices sees nothing, and the block the row joins is singular by
// construction:
//
//   singular Jacobian in algebraic loop ["smee.airGap.i_sr[2]", ...]
//
// With `OXIDELICA_SUPPLY_ONCE=4` a member is refused when its rows,
// read linearly with the coefficients the parameters fold to, are a
// combination of rows already held, and the reduction goes on to the
// next member. Nine members are refused that way and the one after
// them is taken: reduction 14 demotes `smee.lssigma.i_[1]` where the
// rule off demotes `loadInductor.inductor[1].i`, and the model runs to
// 0.01 s with load currents 0.013147, -0.005780 and -0.007367 A,
// summing to zero. `scripts/victim_witness.sh` asks both halves of one
// binary. An opt-in: it changes the reductions of every model it
// touches (docs/ANALYSIS.md, m407).
model a_star_sum_stated_again_through_other_equations
  "Test example: ElectricalExcitedSynchronousMachine with voltage controller"
  import Modelica.Electrical.Machines;
  import Modelica.Units.SI;
  
  import Modelica.Constants.pi;
  constant Integer m=3 "Number of phases";
  parameter SI.AngularVelocity wNominal=2*pi*smeeData.fsNominal
      /smee.p "Nominal speed";
  parameter SI.Impedance ZNominal=3*smeeData.VsNominal^2/
      smeeData.SNominal "Nominal load impedance";
  parameter Real powerFactor(
    min=0,
    max=1) = 0.8 "Load power factor";
  parameter SI.Resistance RLoad=ZNominal*powerFactor
    "Load resistance";
  parameter SI.Inductance LLoad=ZNominal*sqrt(1 -
      powerFactor^2)/(2*pi*smeeData.fsNominal) "Load inductance";
  parameter SI.Voltage Ve0=smee.IeOpenCircuit*
      Machines.Thermal.convertResistance(
            smee.Re,
            smee.TeRef,
            smee.alpha20e,
            smee.TeOperational) "No load excitation voltage";
  parameter Real k=2*Ve0/smeeData.VsNominal "Voltage controller: gain";
  parameter SI.Time Ti=smeeData.Td0Transient/2
    "Voltage controller: integral time constant";
  Machines.BasicMachines.SynchronousMachines.SM_ElectricalExcited smee(
    fsNominal=smeeData.fsNominal,
    Rs=smeeData.Rs,
    TsRef=smeeData.TsRef,
    Lssigma=smeeData.Lssigma,
    Lmd=smeeData.Lmd,
    Lmq=smeeData.Lmq,
    Lrsigmad=smeeData.Lrsigmad,
    Lrsigmaq=smeeData.Lrsigmaq,
    Rrd=smeeData.Rrd,
    Rrq=smeeData.Rrq,
    TrRef=smeeData.TrRef,
    VsNominal=smeeData.VsNominal,
    IeOpenCircuit=smeeData.IeOpenCircuit,
    Re=smeeData.Re,
    TeRef=smeeData.TeRef,
    sigmae=smeeData.sigmae,
    useDamperCage=true,
    p=2,
    Jr=0.29,
    Js=0.29,
    statorCoreParameters(VRef=100),
    strayLoadParameters(IRef=100),
    brushParameters(ILinear=0.01),
    TsOperational=293.15,
    alpha20s=smeeData.alpha20s,
    TrOperational=293.15,
    alpha20r=smeeData.alpha20r,
    alpha20e=smeeData.alpha20e,
    TeOperational=293.15)
    ;
  parameter Machines.Utilities.SynchronousMachineData smeeData(
    SNominal=30e3,
    VsNominal=100,
    fsNominal=50,
    IeOpenCircuit=10,
    x0=0.1,
    xd=1.6,
    xq=1.6,
    xdTransient=0.1375,
    xdSubtransient=0.121428571,
    xqSubtransient=0.148387097,
    Ta=0.014171268,
    Td0Transient=0.261177343,
    Td0Subtransient=0.006963029,
    Tq0Subtransient=0.123345081,
    TsSpecification=293.15,
    TsRef=293.15,
    alpha20s(displayUnit="1/K") = Modelica.Electrical.Machines.Thermal.Constants.alpha20Zero,
    TrSpecification=293.15,
    TrRef=293.15,
    alpha20r(displayUnit="1/K") = Modelica.Electrical.Machines.Thermal.Constants.alpha20Zero,
    TeSpecification=293.15,
    TeRef=293.15,
    alpha20e(displayUnit="1/K") = Modelica.Electrical.Machines.Thermal.Constants.alpha20Zero)
    "Synchronous machine data"
    ;

  Machines.Utilities.TerminalBox terminalBox(terminalConnection="Y")
    ;
  Modelica.Electrical.Analog.Basic.Ground ground ;
  Modelica.Mechanics.Rotational.Sources.Speed speed
    ;
  Modelica.Blocks.Sources.Ramp speedRamp(height=wNominal, duration=1)
    ;
  Modelica.Mechanics.Rotational.Sensors.SpeedSensor speedSensor
    ;
  Modelica.Blocks.Math.Gain setPointGain(k=(smeeData.VsNominal/wNominal))
    ;
  Machines.Sensors.VoltageQuasiRMSSensor voltageQuasiRMSSensor(ToSpacePhasor1(y(
          each start=1E-3, each fixed=true))) ;
  Modelica.Blocks.Continuous.LimPID voltageController(
    controllerType=Modelica.Blocks.Types.SimpleController.PI,
    k=k,
    Ti=Ti,
    yMax=2.5*Ve0,
    yMin=0,
    initType=Modelica.Blocks.Types.Init.InitialState,
    Td=0.001)
    ;
  Modelica.Electrical.Analog.Sources.SignalVoltage excitationVoltage
    ;
  Modelica.Electrical.Analog.Basic.Ground groundExcitation ;
  Modelica.Electrical.Polyphase.Basic.Resistor loadResistor(m=m, R=fill(
        RLoad, m))
    ;
  Modelica.Electrical.Polyphase.Basic.Inductor loadInductor(m=m, L=fill(
        LLoad, m))
    ;
  Modelica.Electrical.Polyphase.Basic.Star star(m=m) ;
protected
  constant SI.MagneticFlux unitMagneticFlux=1
    ;
initial equation
  smee.idq_sr = zeros(2);
  //conditional damper cage currents are defined as fixed start values
  smee.ie = 0;
equation
  connect(terminalBox.plug_sn, smee.plug_sn) ;
  connect(terminalBox.plug_sp, smee.plug_sp) ;
  connect(excitationVoltage.p, smee.pin_ep) ;
  connect(excitationVoltage.n, smee.pin_en) ;
  connect(excitationVoltage.n, groundExcitation.p) ;
  connect(voltageQuasiRMSSensor.plug_n, smee.plug_sn) ;
  connect(voltageQuasiRMSSensor.plug_p, smee.plug_sp) ;
  connect(terminalBox.plugSupply, loadResistor.plug_p)
    ;
  connect(smee.flange, speed.flange) ;
  connect(speed.flange, speedSensor.flange) ;
  connect(speedRamp.y, speed.w_ref) ;
  connect(setPointGain.y, voltageController.u_s) ;
  connect(speedSensor.w, setPointGain.u) ;
  connect(voltageQuasiRMSSensor.V, voltageController.u_m) ;
  connect(voltageController.y, excitationVoltage.v) ;
  connect(loadInductor.plug_p, loadResistor.plug_n) ;
  connect(star.plug_p, loadInductor.plug_n) ;
  connect(star.pin_n, ground.p) ;
  annotation(experiment(StopTime=0.01));
end a_star_sum_stated_again_through_other_equations;
