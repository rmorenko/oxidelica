package Poly "Array-valued helpers for the example below"
  function powers "The first n powers of x, starting at x^0"
    input Real x;
    input Integer n;
    output Real p[4];
  algorithm
    p[1] := 1;
    for i in 2:4 loop
      p[i] := p[i - 1] * x;
    end for;
  end powers;

  function horner "Evaluate a cubic given by its coefficients"
    input Real c[4];
    input Real x;
    output Real y;
  algorithm
    y := ((c[4] * x + c[3]) * x + c[2]) * x + c[1];
  end horner;
end Poly;

model PolynomialFit "A cubic evaluated two ways that must agree exactly"
  parameter Real c[4] = {2.0, -3.0, 0.5, 1.25} "coefficients, lowest first";
  Real x "the moving argument";
  Real by_horner "the cubic, evaluated by Horner's rule";
  Real by_powers "the same cubic, as a scalar product with the powers";
  Real disagreement "identically zero when both routes agree";
equation
  x = 2 * sin(time);
  by_horner = Poly.horner(c, x);
  // powers() returns a whole array; the scalar product folds it with
  // the coefficients.
  by_powers = c * Poly.powers(x, 4);
  disagreement = by_horner - by_powers;
  annotation(experiment(StopTime = 6.283185307, Interval = 0.01));
end PolynomialFit;
