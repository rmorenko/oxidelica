// The ring of `a_ring_closed_through_a_pass_through.mo` again, with
// the wrapper holding an ARRAY of pass-throughs rather than one. That
// is the shape of `PlugToPins_p` in the quasi-static libraries, which
// joins one outer plug to `m` inner adapters, each of which writes a
// `Connections.branch` of its own.
//
// An array multiplies the rings: the outer pin is joined to all three
// adapters, so two of those three connections close a loop and owe the
// record's constraint rather than an equality. What the spanning tree
// must not do is lose an element's index on the way - if
// `pass[1].pin_p.reference` and `pass[2].pin_p.reference` were read as
// one node, the connections of different elements would look like a
// repeat of one edge and the whole array would collapse to a single
// branch, taking equalities away that the model needs.
//
// The angle travels down every element, so `pass[k].load.omega` is 2
// for each of the three, and the angle is 0.002 at t = 0.001.

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
    pin_p.v - pin_n.v = 3 * pin_p.i;
    pin_p.i + pin_n.i = 0;
  end Load;

  // One pass-through: no branch of its own, the inner load draws the
  // edge and these two pins reach the graph through the connections.
  model Pass
    Pin pin_p;
    Pin pin_n;
    Load load;
  equation
    connect(pin_p, load.pin_p);
    connect(load.pin_n, pin_n);
  end Pass;

  // Three of them in parallel between one pair of outer pins.
  model Wrapper
    Pin pin_p;
    Pin pin_n;
    Pass pass[3];
  equation
    for k in 1:3 loop
      connect(pin_p, pass[k].pin_p);
      connect(pass[k].pin_n, pin_n);
    end for;
  end Wrapper;

  model Ground
    Pin pin;
  equation
    pin.v = 0;
  end Ground;

  model Test
    Source source;
    Wrapper wrapper;
    Ground ground;
  equation
    connect(source.pin_p, wrapper.pin_p);
    connect(wrapper.pin_n, source.pin_n);
    connect(source.pin_n, ground.pin);
  end Test;
end QS;
