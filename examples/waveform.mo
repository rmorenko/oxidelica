model Waveform "A signal whose shape an enumeration picks, through a first-order lag"
  import Oxidelica.Blocks;
  import Oxidelica.Types;

  // Sine, Square or Triangle: the source has one equation for all three
  // and chooses on this parameter.
  parameter Types.WaveformKind kind = Types.WaveformKind.Square "shape of the signal";

  Blocks.Sources.Waveform source(kind = kind, amplitude = 1.0, f = 0.5);
  Blocks.Continuous.FirstOrder lag(k = 1.0, T = 0.3, y_start = 0.0);

  Real u "the signal itself";
  Real y "the signal after the lag";
equation
  lag.u = source.y;
  u = source.y;
  y = lag.y;
  annotation(experiment(StopTime = 4.0, Interval = 0.002, Tolerance = 1e-9));
end Waveform;
