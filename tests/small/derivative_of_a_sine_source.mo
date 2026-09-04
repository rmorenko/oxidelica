// Squeezed down from Modelica.Electrical.Analog.Examples.ResonanceCircuits.
//
// The library writes `Modelica.Math.sin`, and what reaches
// differentiation is `.sin` - the name with its package path resolved
// away to nothing. A derivative rule matched against the whole name
// found no rule for that, so an inductor driven by a sine source was
// refused as `structurally singular ... (cannot differentiate function
// `.sin`)`. Twenty-six models were refused for a rule the compiler has.
model DerivativeOfASineSource
  Modelica.Electrical.Analog.Sources.SineCurrent src(I = 1, f = 50);
  Modelica.Electrical.Analog.Basic.Inductor l(L = 1, i(fixed = true, start = 0));
  Modelica.Electrical.Analog.Basic.Ground g;
equation
  connect(src.p, l.p);
  connect(l.n, src.n);
  connect(src.n, g.p);
  annotation(experiment(StopTime = 1));
end DerivativeOfASineSource;
