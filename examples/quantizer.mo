model Quantizer "A sampled signal pushed through the staircase of an ADC"
  import Oxidelica.Blocks;
  import Oxidelica.Constants;

  parameter Real amplitude = 1.0 "amplitude of the analog signal";
  parameter Real f = 1.0 "frequency in Hz";
  parameter Real step = 0.25 "height of one quantization step";
  parameter Integer levels = 4 "steps above zero, and as many below";
  parameter Real Ts = 0.05 "sampling period of the converter";

  // The sampler holds its value between ticks, so everything after it
  // is a staircase in time as well as in amplitude.
  Blocks.Discrete.Sampler adc(samplePeriod = Ts);

  Real signal "the analog signal";
  Real quantized "the sample after the staircase";
  Real error "what the conversion threw away";
equation
  signal = amplitude * sin(2 * Constants.pi * f * time);
  adc.u = signal;
  error = adc.y - quantized;
algorithm
  // Walk the levels and keep the highest one the sample has passed, in
  // both directions: a loop and a branch that the compiler turns into a
  // single equation for `quantized`.
  quantized := 0.0;
  for i in 1:levels loop
    if adc.y > i * step then
      quantized := i * step;
    end if;
    if adc.y < -i * step then
      quantized := -i * step;
    end if;
  end for;
  annotation(experiment(StopTime = 3.0, Interval = 0.005, Tolerance = 1e-9));
end Quantizer;
