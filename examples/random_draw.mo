model NoiseDraw "A generator whose body is written outside Modelica"
  // `xorshift64*` is Vigna's, and the standard library declares it in
  // Modelica and writes it in C. This compiler writes it again in Rust,
  // so a model reaching for it needs nothing brought along - and draws
  // the same stream any other tool would from the same seed.
  package Gen
    constant Integer nState = 2;
    pure function random "One draw, and the state it moved to"
      input Integer stateIn[nState];
      output Real result;
      output Integer stateOut[nState];
      external "C" ModelicaRandom_xorshift64star(stateIn, stateOut, result);
    end random;
  end Gen;
  Integer state[2](start = {126247697, 0}, each fixed = true)
    "The generator's state, carried across events";
  discrete Real r(start = 0, fixed = true) "The number last drawn";
equation
  when sample(0, 0.25) then
    (r, state) = Gen.random(pre(state));
  end when;
  annotation(experiment(StopTime = 1, Interval = 0.25, Tolerance = 1e-10));
end NoiseDraw;
