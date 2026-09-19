// A short connector definition written inside a package is a class of
// its own, exactly as the same line written as a whole file is. Kept
// only as a local name of the package, resolving it walked straight
// through to what it stands for - `connector Din = input Dsig` arrives
// at `Dsig`, which says nothing about a direction - and the `input`
// the definition wrote was lost on the way.
//
// What a connection set does with a one-value connector turns on
// exactly that direction: an `input` joined to nothing takes its own
// start, and a set of two is written from the `output` end towards the
// `input` one. With no direction at all, neither end is the source, no
// equation is written, and the model is refused as
//
//   unbalanced model: 1 algebraic equation(s) for 2 unknown(s)
//
// The same three lines written at the top of a file worked, which is
// the tell: the file road minted a class and the package road did not.
//
// This is what the whole Electrical.Digital family stood at.

package Q
  type Logic = enumeration(U, X);
  connector Dsig = Logic;
  connector Din = input Dsig;
  connector Dsnk = output Dsig;
end Q;

model Snk
  Q.Din x;
  Q.Dsnk y;
equation
  y = x;
end Snk;

model AShortConnectorInsideAPackage
  Snk k;
end AShortConnectorInsideAPackage;
