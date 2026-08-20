model TableRamp "A table of the standard library, read where the model wrote it"
  // `[0, 0; 1, 2; 2, 6]` is two straight lines: a slope of two up to
  // one, then a slope of four. The block keeps its data in a handle
  // and asks C for a value; this compiler writes the lines out instead,
  // so at t = 1.5 the answer is 2 + 0.5 * 4 = 4.
  Modelica.Blocks.Tables.CombiTable1Ds t(table = [0, 0; 1, 2; 2, 6]);
  Real y "The table read at the current time";
equation
  t.u = time;
  y = t.y[1];
  annotation(experiment(StopTime = 1.5, Interval = 0.5, Tolerance = 1e-10));
end TableRamp;
