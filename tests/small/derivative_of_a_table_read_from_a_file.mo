// Squeezed down from ModelicaTest.Tables.CombiTimeTable.Test55 by
// dropping components while the refusal stayed, which is how it was
// found: three invented models all flattened, because the cause was
// not in them.
//
// A table read from a file becomes one `if` per interval, nested - the
// file here is a hundred rows, so the expression is ninety-nine deep.
// Differentiating it walked that depth and hit a limit meant for
// cyclic definitions, and the model was refused as `structurally
// singular ... (differentiation recursed through a cyclic definition)`
// for a cycle that does not exist.
//
// A table written out in the model flattens either way: it is short.
// The file is the whole difference.
model DerivativeOfATableReadFromAFile
  Modelica.Blocks.Sources.CombiTimeTable t_new(
    tableOnFile = true,
    tableName = "a",
    fileName = Modelica.Utilities.Files.loadResource(
      "modelica://Modelica/Resources/Data/Tables/test_v4.mat"));
  Real y;
equation
  y = der(t_new.y[1]);
  annotation(experiment(StopTime = 100));
end DerivativeOfATableReadFromAFile;
