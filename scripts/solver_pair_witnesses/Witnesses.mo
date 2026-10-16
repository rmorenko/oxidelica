within;
package Witnesses
  "Models where a lost crossing shows, held beside the library models in the solver-pair reference"
  // Defect 7: a relation on a smooth signal beside a state whose error
  // estimate lets the step grow past the time the relation spends on
  // the other side of its threshold. The library examples did not show
  // it - the reference of 330 held whole under the switch that puts the
  // defect back - so these are what the reference stands on for it.
  model Rises "when sin(20 t) > 0.5 beside der(x) = 1: four rises"
    Real x(start = 0, fixed = true);
    discrete Real m(start = 0, fixed = true);
  equation
    der(x) = 1;
    when sin(20*time) > 0.5 then
      m = pre(m) + 1;
    end when;
    annotation(experiment(StopTime = 1, Interval = 0.01));
  end Rises;

  model RisesLonger "when sin(10 t) > 0.5 over four seconds: seven rises"
    Real x(start = 0, fixed = true);
    discrete Real m(start = 0, fixed = true);
  equation
    der(x) = 1;
    when sin(10*time) > 0.5 then
      m = pre(m) + 1;
    end when;
    annotation(experiment(StopTime = 4, Interval = 0.01));
  end RisesLonger;

  model Dips "when cos(20 t) < -0.5: three falls, each right after a cut step"
    Real x(start = 0, fixed = true);
    discrete Real m(start = 0, fixed = true);
  equation
    der(x) = 1;
    when cos(20*time) < -0.5 then
      m = pre(m) + 1;
    end when;
    annotation(experiment(StopTime = 1, Interval = 0.01));
  end Dips;

  model Threshold "Sine through GreaterThreshold into an Integrator: 0.3455"
    Modelica.Blocks.Sources.Sine sine(f = 20/(2*3.141592653589793), amplitude = 1);
    Modelica.Blocks.Logical.GreaterThreshold gt(threshold = 0.5);
    Modelica.Blocks.Continuous.Integrator integ(k = 1);
    Modelica.Blocks.Math.BooleanToReal b2r;
  equation
    connect(sine.y, gt.u);
    connect(gt.y, b2r.u);
    connect(b2r.y, integ.u);
    annotation(experiment(StopTime = 1, Interval = 0.01));
  end Threshold;

  model Hysteresis "Sine through Hysteresis into an Integrator: 0.5026"
    Modelica.Blocks.Sources.Sine sine(f = 20/(2*3.141592653589793));
    Modelica.Blocks.Logical.Hysteresis h(uLow = -0.5, uHigh = 0.5);
    Modelica.Blocks.Math.BooleanToReal b2r;
    Modelica.Blocks.Continuous.Integrator integ(k = 1);
  equation
    connect(sine.y, h.u);
    connect(h.y, b2r.u);
    connect(b2r.y, integ.u);
    annotation(experiment(StopTime = 1, Interval = 0.01));
  end Hysteresis;
end Witnesses;
