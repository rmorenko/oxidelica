// Squeezed down from ModelicaTest.Media.TestAllProperties.IncompleteMedia.ReferenceAir_dT.
//
// `prandtlNumber` folds the Helmholtz calls shallow, and the table of
// bodies already inlined hands the same answers back deeper down, inside
// `velocityOfSound`. What that call builds is then too deep to read once
// more, and it is left standing for the run to walk. It was left
// standing under the name of the class that wrote it, `Air_Base`, rather
// than under the copy carried for `Air_dT`, and the walk stopped on
// `unknown variable dT_explicit` - a constant only the medium gives.
//
// With the fault fixed it runs and gives a = 346.291922, the value the
// same three lines give with `a` written first. OXIDELICA_STANDING_WRITER=1
// brings the fault back. No model small enough to write here reaches
// the path: it switches on only past the depth the compiler follows, so
// the corpus pair is the witness and this file the cheap check.
model ACallTooDeepToReadAgainUnderItsMedium
  package Medium = Modelica.Media.Air.ReferenceAir.Air_dT;
  Medium.ThermodynamicState state = Medium.setState_pTX(Medium.reference_p, Medium.reference_T, Medium.reference_X);
  Medium.PrandtlNumber Pr = Medium.prandtlNumber(state);
  Medium.VelocityOfSound a = Medium.velocityOfSound(state);
  annotation(experiment(StopTime = 0.01));
end ACallTooDeepToReadAgainUnderItsMedium;
