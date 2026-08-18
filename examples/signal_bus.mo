model SignalBus "a control loop wired through an expandable bus"
  // The bus declares nothing. Every member below - `measurement`,
  // `command` - exists because a connection named it, and the sub-bus
  // shares the pool because it is connected to the bus itself. The
  // loop that comes out is der(x) = (k(r - x) - x)/T, which settles at
  // k*r/(1+k) = 0.8 with time constant T/(1+k) = 0.1 s.
  connector RealOutput output Real y "a signal leaving a component";
  end RealOutput;

  connector RealInput input Real y "a signal entering a component";
  end RealInput;

  expandable connector Bus "holds whatever is connected to it"
  end Bus;

  model Plant "a first-order lag driven by its input"
    parameter Real T = 0.5 "time constant";
    RealInput u;
    RealOutput y;
    Real x(start = 0) "the state that is being controlled";
  equation
    der(x) = (u.y - x) / T;
    y.y = x;
  end Plant;

  model Controller "a proportional law"
    parameter Real k = 4 "gain";
    parameter Real setpoint = 1;
    RealInput measurement;
    RealOutput command;
  equation
    command.y = k * (setpoint - measurement.y);
  end Controller;

  Bus bus;
  Bus subbus "joined to the bus, so it carries the same members";
  Plant plant(T = 0.5);
  Controller controller(k = 4, setpoint = 1);
equation
  connect(bus, subbus);
  connect(plant.y, bus.measurement);
  connect(controller.command, bus.command);
  connect(subbus.measurement, controller.measurement);
  connect(subbus.command, plant.u);
  annotation(experiment(StopTime = 1.0, Interval = 0.005, Tolerance = 1e-10));
end SignalBus;
