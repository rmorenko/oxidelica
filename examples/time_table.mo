model TimeRamp "A table of the standard library whose first column is time"
  // The block keeps its data in a handle and asks C for a value at
  // every step; this compiler writes the lines out instead. Two
  // straight lines - a slope of two up to t = 1, then a slope of four -
  // so at t = 1.5 the answer is 2 + 0.5 * 4 = 4. The corners become
  // events, which is what `nextTimeEvent` is for.
  Modelica.Blocks.Sources.CombiTimeTable t(table = [0, 0; 1, 2; 2, 6]);
  Real y "The table read at the current time";
equation
  y = t.y[1];
  annotation(experiment(StopTime = 1.5, Interval = 0.5, Tolerance = 1e-10));
end TimeRamp;
