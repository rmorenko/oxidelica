// A record handed whole to a declaration typed as its base. This is
// the shape `ShowImpedance` is written in: `Impedance` declares its
// parameter as `CellData` and the example hands it an `ExampleData`,
// which extends `CellData` and so holds more fields than the base
// declares.
//
// The hand-over was matched by position and by count - the value taken
// apart by the class that wrote it, the names taken from the class
// receiving it. Those are the same class most of the time and were not
// here, so the counts disagreed and the whole hand-over was dropped in
// silence. What the model then said was `parameter c2.Qnom has no
// value`, though `Qnom = 3600` stands in plain sight two lines up.
//
// With the fix the field the target calls `Qnom` takes the value at
// `cellData.Qnom` by name, and the model runs.
model M
  record Bas
    parameter Real Qnom;
  end Bas;
  record Der
    extends Bas;
    parameter Real Ri = 0.5;
  end Der;
  parameter Der cellData(Qnom = 3600, Ri = 0.01);
  parameter Bas c2 = cellData;
  Real y(start = 0, fixed = true);
equation
  der(y) = c2.Qnom - y;
  annotation(experiment(StopTime = 1));
end M;
