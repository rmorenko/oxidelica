// Squeezed down from Modelica.Blocks.Examples.Noise.Utilities.ImpureRandom.
//
// A helper class of a library is written to sit inside a model that
// holds the shared instance, and says `outer GlobalSeed globalSeed` on
// that understanding. Checked on its own it has nothing above it at
// all, and the declaration answers to nobody: the compiler refused it
// as `outer Seed s in Helper has no inner declaration above it`.
//
// The language says what to do instead (MLS 5.4): declare the missing
// `inner` at the top of the model with its class's own defaults, and
// say so. Thirteen models of the standard library stood at that wall,
// every one of them a `Utilities` or `BaseClasses` helper.
model Seed
  parameter Real id = 3.0;
end Seed;

model Helper
  outer Seed s;
  Real y;
equation
  y = s.id;
end Helper;

model AnOuterWithNoInnerAboveTheTopModel
  Helper h;
  annotation(experiment(StopTime = 1));
end AnOuterWithNoInnerAboveTheTopModel;
