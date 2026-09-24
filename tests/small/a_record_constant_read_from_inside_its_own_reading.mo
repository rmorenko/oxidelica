// A record constant read from inside its own reading. This is
// `Modelica.Media.Examples.SolveOneNonlinearEquation.Inverse_sh_TX`
// cut to its first equation: the mixture gases' `T_hX` hands Brent's
// method `data = data`, and reading that record's binding inlines a
// function that is handed the same bare name again.
//
// Once a specialized copy took its array input whole, the call went on
// to be inlined, and the reading asked the same question one storey
// down, for ever. A corpus pass held on this model for half an hour
// with one thread busy and nothing printed.
//
// With the guard the inner reading answers what it answered before
// the road existed, and the model is refused in seconds - today for
// `referenceChoice`, a name of the medium no flat model declares.
model M
  package Medium = Modelica.Media.IdealGases.MixtureGases.FlueGasLambdaOnePlus;
  parameter Real X[4] = Medium.reference_X;
  Real h1 = 3e5 + 1e5*time;
  Real Th;
equation
  Th = Medium.temperature_phX(1e5, h1, X);
  annotation(experiment(StopTime = 1));
end M;
