// A parameter whose value is a solve of a matrix written out in full.
// This is the shape the pump characteristics of the standard library
// are written in: `quadraticFlow` fits a quadratic through three
// operating points, and `Modelica.Math.Matrices.solve` reaches the
// flat model as a bare `dgesv` over a three by three matrix.
//
// `dgesv` has had a body written here in Rust for as long as
// `outside.rs` has existed, so nothing was missing from the library of
// bodies. What refused was the taking apart of the argument: a body
// written here takes numbers, a matrix arrives as an array of rows,
// and the side of the run that settles parameters before the run
// matched `Expr::Array` once and evaluated its items. Three rows were
// offered where nine numbers were wanted, the shape did not fit, and
// the name came back as `nothing works out `dgesv``  -  a refusal
// whose wording pointed at the body rather than at the caller, which
// is why the cluster read for several shifts as needing an
// interpreter for function bodies.
//
// With the fix both sides of the run walk an argument to its leaves
// through one function, and the numbers come out right: the quadratic
// through (0, 100), (0.25, 60) and (0.5, 0) has the coefficients 100,
// -120 and -160, so `k` is -120 and `x` settles towards it.
model M
  parameter Real c[3] = Modelica.Math.Matrices.solve(
    [1, 0, 0; 1, 0.25, 0.0625; 1, 0.5, 0.25], {100, 60, 0});
  parameter Real k = c[2];
  Real x;
equation
  der(x) = -x + k;
  annotation(experiment(StopTime = 1));
end M;
