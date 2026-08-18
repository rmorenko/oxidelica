model IdealRectifier "a half-wave rectifier with a textbook ideal switch"
  // The switch is written the way the textbooks write an ideal one:
  // blocking is an equation on the current, conducting an equation on
  // the voltage. The two branches constrain *different* unknowns,
  // which no `if` expression can express - a value may be chosen by a
  // condition, an unknown may not.
  //
  // Each mode is compiled as its own model, so the matching of
  // equations to unknowns is made for the mode in force and made again
  // at the instant it switches. With a resistive load a diode conducts
  // exactly while the supply pushes forward, so commanding the switch
  // on the supply's polarity is the diode's own behaviour - and it
  // keeps the condition out of the branches' hands. A condition on the
  // switch's own `v` could not: while conducting, `v` is held at zero
  // by the very branch that would be tested, and the switch could
  // never open again.
  connector Pin
    Real v "potential";
    flow Real i "current into the pin";
  end Pin;

  connector RealOutput
    output Real y;
  end RealOutput;

  connector RealInput
    input Real y;
  end RealInput;

  model IdealSwitch "blocks the current, or conducts with no drop"
    Pin p;
    Pin n;
    RealInput command "positive asks it to conduct";
    Real v "voltage across, p minus n";
    Real i "current through, p to n";
    Boolean blocking "true while it holds the current off";
  equation
    v = p.v - n.v;
    i = p.i;
    p.i + n.i = 0;
    blocking = command.y < 0;
    if blocking then
      i = 0 "no current while it blocks";
    else
      v = 0 "no drop while it conducts";
    end if;
  end IdealSwitch;

  model Resistor
    parameter Real R = 2;
    Pin p;
    Pin n;
  equation
    p.v - n.v = R * p.i;
    p.i + n.i = 0;
  end Resistor;

  model Supply "a sine that goes both ways, so the switch has work to do"
    parameter Real amplitude = 10;
    parameter Real frequency = 1;
    Pin p;
    Pin n;
    RealOutput y "what it is putting out, for whoever needs to know";
    Real v;
  equation
    v = amplitude * sin(6.283185307179586 * frequency * time);
    p.v - n.v = v;
    p.i + n.i = 0;
    y.y = v;
  end Supply;

  model Ground
    Pin p;
  equation
    p.v = 0;
  end Ground;

  Supply supply(amplitude = 10, frequency = 1);
  IdealSwitch switch;
  Resistor load(R = 2);
  Ground ground;
  Real clipped "the analytic answer, for comparison";
equation
  connect(supply.p, switch.p);
  connect(supply.y, switch.command);
  connect(switch.n, load.p);
  connect(load.n, ground.p);
  connect(supply.n, ground.p);
  clipped = max(supply.v, 0) / 2;
  annotation(experiment(StopTime = 2, Interval = 0.002, Tolerance = 1e-10));
end IdealRectifier;
