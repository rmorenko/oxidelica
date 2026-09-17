// A record handed whole to a component, whose own array is as long as
// a field of that record says. `cell(cellData = cellData)` is how the
// battery library passes its parameters down, and `rcData[nRC]` is the
// array whose length `nRC` gives.
//
// The fields of the receiving record were settled from the record's
// own defaults alone, so `nRC` came out as the declaration's 1 rather
// than the 2 the site handed in, and the array below it was built one
// element long. What the model then said was that nothing gives a
// value to `cell.cellData.a.R` - a name with no subscript, which is
// what a slice of an array that was never built reaches the run as.
//
// With the fix the length comes from the value the site wrote, the
// array is built with both elements, and the model runs.
model M
  record Elem
    parameter Real R = 1;
  end Elem;
  record CellData
    parameter Integer n = 1;
    parameter Elem a[n] = {Elem(R = 0)};
  end CellData;
  model Res
    parameter Real R = 0;
    Real i;
  equation
    i = R;
  end Res;
  model Stack
    parameter CellData cellData;
    Res res[cellData.n](final R = cellData.a.R);
  end Stack;
  parameter CellData cellData(n = 2, a = {Elem(R = 1), Elem(R = 2)});
  Stack cell(cellData = cellData);
  Real y(start = 0, fixed = true);
equation
  der(y) = cell.res[2].i - y;
  annotation(experiment(StopTime = 1));
end M;
