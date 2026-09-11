// The same ring as `a_ring_of_the_overconstrained_graph.mo`, but one
// side of it reaches the ring through a component that writes no
// `Connections.branch` of its own: a wrapper whose two pins are joined
// to an inner component's pins by plain `connect` equations. That is
// the shape of `PlugToPins_p` in the quasi-static libraries, where the
// outer plug is passed through to an array of inner converters.
//
// The nodes of the overconstrained graph used to be gathered from the
// `branch` and `root` clauses alone, so the wrapper's own pins were no
// nodes at all: the connections naming them wrote blanket equalities
// and, worse, did not extend the spanning tree - so the connection
// that really closes the ring was never recognised as closing it. The
// model came out one equation over.
//
// Every connector of a set holding a record with an empty
// `equalityConstraint` is a node of the graph whether a branch names it
// or not, which is what makes the wrapper's pins carry the tree.
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

  // No branch of its own: the inner load is what draws the edge, and
  // these two pins reach the graph only through the connections.
  model Wrapper
    Pin pin_p;
    Pin pin_n;
    Load load;
  equation
    connect(pin_p, load.pin_p);
    connect(load.pin_n, pin_n);
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
