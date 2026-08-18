model ComplexImpedance "an RLC circuit meets its own phasor"
  // What overloaded operators are for: `Z = R + j * (wL - 1/(wC))` is
  // written the way it is written on paper, and the compiler works out
  // that `+`, `*` and `/` here mean the record's own.
  //
  // The phasor says what the steady state must be. The circuit is then
  // integrated from rest until the transient has died, and the two are
  // compared - amplitude and phase both.
  operator record Complex "a number with two parts"
    Real re;
    Real im;

    encapsulated operator function '+'
      input Complex c1;
      input Complex c2;
      output Complex c3;
    algorithm
      c3 := Complex(c1.re + c2.re, c1.im + c2.im);
    end '+';

    encapsulated operator '-'
      function negate
        input Complex c1;
        output Complex c2;
      algorithm
        c2 := Complex(-c1.re, -c1.im);
      end negate;

      function subtract
        input Complex c1;
        input Complex c2;
        output Complex c3;
      algorithm
        c3 := Complex(c1.re - c2.re, c1.im - c2.im);
      end subtract;
    end '-';

    encapsulated operator function '*'
      input Complex c1;
      input Complex c2;
      output Complex c3;
    algorithm
      c3 := Complex(
        c1.re * c2.re - c1.im * c2.im,
        c1.re * c2.im + c1.im * c2.re);
    end '*';

    encapsulated operator function '/'
      input Complex c1;
      input Complex c2;
      output Complex c3;
    protected
      Real d;
    algorithm
      d := c2.re * c2.re + c2.im * c2.im;
      c3 := Complex(
        (c1.re * c2.re + c1.im * c2.im) / d,
        (c1.im * c2.re - c1.re * c2.im) / d);
    end '/';
  end Complex;

  parameter Real R = 2 "resistance";
  parameter Real L = 0.5 "inductance";
  parameter Real C = 0.1 "capacitance";
  parameter Real w = 4 "drive frequency";
  parameter Real V = 10 "drive amplitude";

  // The phasor, in complex arithmetic.
  Complex j "the imaginary unit";
  Complex reactance "wL - 1/(wC), as a number with two parts";
  Complex impedance "R + j * reactance";
  Complex current "V / Z, the phasor of the steady-state current";
  Real predicted_amplitude;
  Real predicted_phase;

  // The circuit itself, integrated from rest.
  Real i(start = 0) "current";
  Real q(start = 0) "charge on the capacitor";
equation
  j = Complex(0, 1);
  reactance = Complex(w * L - 1 / (w * C), 0);
  impedance = Complex(R, 0) + j * reactance;
  current = Complex(V, 0) / impedance;
  predicted_amplitude = sqrt(current.re * current.re + current.im * current.im);
  predicted_phase = atan2(current.im, current.re);

  der(q) = i;
  der(i) = (V * sin(w * time) - R * i - q / C) / L;
  annotation(experiment(StopTime = 12, Interval = 0.001, Tolerance = 1e-10));
end ComplexImpedance;
