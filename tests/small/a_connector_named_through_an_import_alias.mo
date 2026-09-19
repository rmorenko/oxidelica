// A connector written as one value - `connector Din = input Dsig` -
// is recognised as a connector by its name rather than by its type,
// since resolving the type leaves the primitive behind and a
// primitive says nothing about being connectable. The name was asked
// of the class holding it, and the holder was looked up by the walk
// out of the enclosing packages alone, which knows nothing of the
// imports.
//
// So a model that writes `import D = Q` and then `D.Din s` named a
// holder nothing found: the connector was not recognised as one, it
// never entered the table of connector instances, and every `connect`
// naming it was refused with
//
//   connect(s, q): both sides must be connector instances
//
// which is what the whole `Electrical.Digital` family stood at - its
// models are written with `import D = Modelica.Electrical.Digital`
// throughout. Written out in full, the same two declarations worked,
// which is the tell: a name that means the same thing gave a
// different answer depending on who spelled it.
//
// Fixed, the connection is made and the value travels: q takes the
// logic value s carries.

package Q
  package Interfaces
    type Logic = enumeration(U, X);
    connector Dsig = Logic;
    connector Din = input Dsig;
    connector Dsink = output Dsig;
  end Interfaces;
end Q;

model AConnectorNamedThroughAnImportAlias
  import D = Q;
  D.Interfaces.Din s;
  D.Interfaces.Dsink q;
equation
  s = D.Interfaces.Logic.X;
  connect(s, q);
end AConnectorNamedThroughAnImportAlias;
