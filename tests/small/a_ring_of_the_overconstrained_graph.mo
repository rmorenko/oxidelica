// A source and a load joined into a ring, both carrying a record with
// an empty equalityConstraint - the shape of the quasi-static
// libraries' reference angle.
//
// Section 9.4: a connect between connectors holding an overconstrained
// record is an edge of a graph, and the connection that closes the ring
// owes the record's equalityConstraint rather than an equality. For a
// residue of no elements that is no equation at all. Written as a
// blanket equality the model is one equation over, and the refusal
// reads:
//
//   unbalanced model: 13 algebraic equation(s) for 12 unknown(s);
//   nothing is left for source.pin_n.reference.gamma = ...
//
// Fixed, it runs, and der(gamma) = 2 makes the angle 0.002 at t = 0.001.

package QS
  record Reference
    Real gamma;
    function equalityConstraint
      input Reference reference1;
      input Reference reference2;
      output Real residue[0];
    algorithm
    end equalityConstraint;
  end Reference;

  connector Pin
    Real v;
    flow Real i;
    Reference reference;
  end Pin;

  model Source
    Pin pin_p;
    Pin pin_n;
    Real gamma(start = 0) = pin_p.reference.gamma;
  equation
    Connections.root(pin_p.reference);
    Connections.branch(pin_p.reference, pin_n.reference);
    pin_p.reference.gamma = pin_n.reference.gamma;
    pin_p.v - pin_n.v = 1;
    pin_p.i + pin_n.i = 0;
    der(gamma) = 2;
  end Source;

  model Load
    Pin pin_p;
    Pin pin_n;
    Real omega;
  equation
    Connections.branch(pin_p.reference, pin_n.reference);
    pin_p.reference.gamma = pin_n.reference.gamma;
    omega = der(pin_p.reference.gamma);
    pin_p.v - pin_n.v = pin_p.i;
    pin_p.i + pin_n.i = 0;
  end Load;

  model Ground
    Pin pin;
  equation
    pin.v = 0;
  end Ground;

  model Test
    Source source;
    Load load;
    Ground ground;
  equation
    connect(source.pin_p, load.pin_p);
    connect(load.pin_n, source.pin_n);
    connect(source.pin_n, ground.pin);
  end Test;
end QS;
