// A check inside a `when`, naming a variable of the component that
// holds it. This is how `MassWithStopAndFriction` guards its hard
// stop, and `Friction` and `HeatLosses` of Translational both die on
// it.
//
// The condition was resolved - which puts the component's prefix on
// its names - and then expanded again, which put the prefix on a
// second time. What reached the run was `stop1.stop1.s`, a name the
// flat model never declares, and the refusal said so without saying
// where the second `stop1` came from.
//
// With the fix the condition is prefixed once and the model runs.
model M
  model Inner
    Real s(start = 0, fixed = true);
  equation
    der(s) = 1;
    when time > 0.5 then
      assert(s > -1, "the mass is past its stop");
    end when;
  end Inner;
  Inner stop1;
  annotation(experiment(StopTime = 1));
end M;
