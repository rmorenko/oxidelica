// A record field whose length is a name, handed to a function through
// an element of an array of records. This is the shape a Fluid pipe
// hands its medium's `density(states[1])` in: the state declares
// `X[nX]`, `nX` is a constant of the medium package, and
// `gasConstant` reads `state.X[Water]`.
//
// The length was measured against an empty table, `nX` answered
// nothing, and the field dropped out of the list bound one by one. What
// the model then said was `unknown variable state.X[1] in equation`: a
// name of the function's own carried out into the flat model. A record
// handed by a plain name was spared only because that road binds the
// bare name as well.
//
// With the fix `X` is bound element by element and R comes to 288.74.
// The same with `nX` a constant of the model rather than of a package
// is still refused: the length is read through the package constants.
model M
  package P
    constant Integer nX = 2;
    record S
      Real p;
      Real X[nX];
    end S;
    function gc
      input S state;
      output Real R;
    algorithm
      R := 287*(1 - state.X[1]) + 461*state.X[1];
    end gc;
  end P;
  P.S st[1](p = {1e5}, X = {{0.01, 0.99}});
  Real R;
equation
  R = P.gc(st[1]);
  annotation(experiment(StopTime = 1));
end M;
