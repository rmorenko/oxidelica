// A quasi-static inductor writes its voltage as `j*omega*L` times the
// current, spelt out over real and imaginary parts, and the real part
// of `j` is a literal `0`. So its equations carry `0 * i.re` and
// `0 * i.im` beside the products that matter.
//
// Quenching every product with a zero factor - which is what reaching
// past `p*der(x)` to `p*x` first did - takes those literal zeros for
// parameters. With a series resistance of 1e-5 ohm the current is
// then held by rows that scale it by the resistance alone, and the
// block of two over `inductor.i` converges to 5.5e-10 against a
// hundred volts and cannot go further. The refusal, under the patch
// `~/oxideflow/state/zero_terms_m316.patch`, reads:
//   the Newton direction of algebraic loop ["inductor.i.im",
//   "inductor.i.re"] does not reduce the residual at t = 0: from
//   |f| = 5.526193272851753e-10
// which is `Modelica.Magnetic.QuasiStatic.FundamentalWave.Examples.
// Components.PolyphaseInductance` shrunk to one phase of its electric
// half (`/tmp/m317/P3.mo`). A resistance of 1e-3 runs either way.
//
// Running, the current at t = 1 is 2.533e-5 real and -15.9155
// imaginary: 100 V over j*2*pi*1 H.

model a_literal_zero_scaling_a_quasi_static_current
  Modelica.Electrical.QuasiStatic.SinglePhase.Basic.Ground ground;
  Modelica.Electrical.QuasiStatic.SinglePhase.Sources.VoltageSource source(
    f = 1, V = 100, phi = 0, gamma(fixed = true, start = 0));
  Modelica.Electrical.QuasiStatic.SinglePhase.Basic.Resistor resistor(R_ref = 1e-5);
  Modelica.Electrical.QuasiStatic.SinglePhase.Basic.Inductor inductor(L = 1);
equation
  connect(source.pin_n, ground.pin);
  connect(source.pin_n, inductor.pin_n);
  connect(source.pin_p, resistor.pin_p);
  connect(resistor.pin_n, inductor.pin_p);
  annotation(experiment(StopTime = 1, Interval = 0.01));
end a_literal_zero_scaling_a_quasi_static_current;
