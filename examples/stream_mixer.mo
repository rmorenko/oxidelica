model StreamMixer "two streams merge at a junction; a tank relaxes to the mix"
  // A stream variable carries what the flow transports. At the
  // three-way node the tank receives the flow-weighted mix of the two
  // sources: (1*100 + 3*20) / 4 = 40, and its contents relax there
  // exponentially with time constant mass / m_flow = 2 s.
  connector Port
    Real p "pressure";
    flow Real m_flow "mass flow into the component";
    stream Real h_outflow "enthalpy carried through the port";
  end Port;

  model Source "pushes a fixed flow at a fixed enthalpy"
    parameter Real m0 = 1 "flow pushed out";
    parameter Real h0 = 100 "enthalpy of what it pushes";
    Port port;
  equation
    port.m_flow = -m0;
    port.h_outflow = h0;
  end Source;

  model Tank "a stirred volume fed through its port"
    parameter Real mass = 8 "contents, which set the time constant";
    parameter Real h_start = 0;
    Real h(start = h_start) "specific enthalpy of the contents";
    Port port;
  equation
    port.p = 1e5;
    port.h_outflow = h;
    der(h) = port.m_flow * (actualStream(port.h_outflow) - h) / mass;
  end Tank;

  Source hot(m0 = 1, h0 = 100);
  Source cold(m0 = 3, h0 = 20);
  Tank tank(mass = 8);
  Real h_supplied "what the junction hands the tank";
equation
  connect(hot.port, tank.port);
  connect(cold.port, tank.port);
  h_supplied = inStream(tank.port.h_outflow);
  annotation(experiment(StopTime = 6, Interval = 0.01, Tolerance = 1e-10));
end StreamMixer;
