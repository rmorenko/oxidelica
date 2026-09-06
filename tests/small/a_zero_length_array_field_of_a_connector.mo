// A connector field that is a zero-length array, connected. A fluid
// port carries `Xi[nXi]`, the independent mass fractions, and a
// single-substance medium (dry air) has `nXi = 0`, so the port has no
// `Xi` to equate. The potential equality the connection would write
// names `port.Xi` on both sides, which no component of the flat model
// is called, and the model was refused `unknown variable`.
//
// The fix skips a connector member the flat model does not carry -
// neither as a scalar nor as any element - so a zero-length array field
// contributes no equation, the way an unconnected flow of one skips a
// name it has not got. Without it this model is refused; with it the
// connection is the two scalars it really is.
package P
  connector Port
    Real p;
    flow Real m;
    Real Xi[0];
  end Port;
  model Src
    Port port;
    Real s(start = 0, fixed = true);
  equation
    port.p = 100;
    der(s) = port.m;
  end Src;
  model Snk
    Port port;
  equation
    port.m = 1;
  end Snk;
  model M
    Src src;
    Snk snk;
  equation
    connect(src.port, snk.port);
    annotation(experiment(StopTime = 1));
  end M;
end P;
