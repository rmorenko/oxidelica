// A specialized copy handed an array on an array input. This is the
// shape `MoistAir.T_phX` is written in: Brent's method is handed
// `function f_nonlinear(p = p, h = h, X = X[1:nXi])`, and the copy made
// of it takes the mass fractions as `Real[:] X`.
//
// The copy is not in the registry, so the call was taken for an
// ordinary scalar function and spread over a vector of one. What the
// model then said was `an equation between shapes [] and [1]` for `T`,
// and in the Fluid examples `an array cannot be a divisor` for the
// density over it.
//
// With the fix the array goes into the copy whole and `T` comes to
// 300000/1010 = 297.0297.
model M
  partial function pf
    input Real u;
    output Real y;
  end pf;
  function solve
    input pf f;
    input Real a;
    input Real b;
    output Real u;
  algorithm
    u := (a + b)/2;
    for i in 1:40 loop
      if f(u) > 0 then b := u; else a := u; end if;
      u := (a + b)/2;
    end for;
  end solve;
  function T_phX
    input Real p;
    input Real h;
    input Real[:] X;
    output Real T;
  protected
    function g
      extends pf;
      input Real p;
      input Real h;
      input Real[:] X;
    algorithm
      y := 1000*u*(1 + X[1]) - h;
    end g;
  algorithm
    T := solve(function g(p=p, h=h, X=X[1:1]), 190, 647);
  end T_phX;
  Real Xi[1] = {0.01};
  Real h = 300000;
  Real T;
  Real d;
equation
  T = T_phX(1e5, h, Xi);
  d = 1e5/(287*T);
  annotation(experiment(StopTime = 1));
end M;
