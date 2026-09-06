// A check inside a `for` equation that reads an array element: the
// media write `for i in 1:nX loop assert(X[i] >= 0, ...) end for` to
// guard their mass fractions. When the loop unrolls, `X[i]` must become
// the element's own name `X[1]`, the same as an equation of the round -
// left as an index into the whole `X`, it reaches the run as a
// subscript that survived flattening and the model is refused.
//
// The assert path in the loop unroll folded the loop variable but did
// not send the condition through the array layer, so the index stood.
// The fix expands it as an equation is expanded.
model AS
  parameter Integer n = 1;
  Real[n] X;
  Real z(start = 0, fixed = true);
equation
  X[1] = 1;
  for i in 1:n loop
    assert(X[i] >= 0, "X out of range");
  end for;
  der(z) = X[1] - z;
  annotation(experiment(StopTime = 1));
end AS;
