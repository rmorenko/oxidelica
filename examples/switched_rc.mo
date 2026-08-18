model SwitchedRC "a chopped supply charging and discharging an RC"
  // A run-time `if` equation: the condition is decided while the model
  // runs, not while it compiles, so both branches stay and each
  // position becomes one equation that picks its residual as it goes.
  // Every relation is an event indicator here, so the switching
  // instants are located exactly rather than stepped over.
  //
  // Chopped at 1 Hz into R*C = 0.2 s, the capacitor charges towards
  // the supply for half a period and discharges towards zero for the
  // next: a staircase of exponentials with an exact closed form.
  connector Pin
    Real v "potential";
    flow Real i "current into the pin";
  end Pin;

  model PulsedSupply "a supply that is chopped on and off"
    parameter Real V = 10 "voltage while energised";
    parameter Real frequency = 1 "chopping rate";
    Pin p;
    Pin n;
    Boolean energised;
    Real delivered "power leaving the supply";
  equation
    p.i + n.i = 0;
    energised = sin(6.283185307179586 * frequency * time) >= 0;
    if energised then
      p.v - n.v = V;
      delivered = V * p.i;
    else
      p.v - n.v = 0;
      delivered = 0;
    end if;
  end PulsedSupply;

  model Resistor
    parameter Real R = 1;
    Pin p;
    Pin n;
  equation
    p.v - n.v = R * p.i;
    p.i + n.i = 0;
  end Resistor;

  model Capacitor
    parameter Real C = 0.2;
    parameter Real v_start = 0;
    Pin p;
    Pin n;
    Real v(start = v_start, fixed = true) "voltage across";
  equation
    v = p.v - n.v;
    der(v) = p.i / C;
    p.i + n.i = 0;
  end Capacitor;

  model Ground
    Pin p;
  equation
    p.v = 0;
  end Ground;

  PulsedSupply supply(V = 10, frequency = 1);
  Resistor resistor(R = 1);
  Capacitor capacitor(C = 0.2, v_start = 0);
  Ground ground;
equation
  connect(supply.p, resistor.p);
  connect(resistor.n, capacitor.p);
  connect(capacitor.n, ground.p);
  connect(supply.n, ground.p);
  annotation(experiment(StopTime = 2, Interval = 0.002, Tolerance = 1e-10));
end SwitchedRC;
