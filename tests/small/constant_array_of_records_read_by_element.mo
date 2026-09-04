// A medium declares its constants as an array of records -
// `constant FluidConstants[nS] fluidConstants` - and a model reads one
// field of one element: `medium.fluidConstants[1].molarMass`. Refused
// as `unknown variable `...fluidConstants[1].molarMass`` where the
// array of constants is never written out as components.
model ConstantArrayOfRecordsReadByElement
  package M
    record FluidConstants
      Real molarMass;
      Real criticalTemperature;
    end FluidConstants;
    constant Integer nS = 1;
    constant FluidConstants fluidConstants[nS] = {FluidConstants(
      molarMass = 0.018,
      criticalTemperature = 647.1)};
    model BaseProperties
      Real MM;
    equation
      MM = fluidConstants[1].molarMass;
    end BaseProperties;
  end M;
  M.BaseProperties medium;
  Real out;
equation
  out = medium.MM*time;
  annotation(experiment(StopTime = 1, Interval = 1));
end ConstantArrayOfRecordsReadByElement;
