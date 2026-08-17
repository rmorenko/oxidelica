package Media
  partial package PartialMedium "What a tank needs to know about a fluid"
    constant Real rho = 0 "density, kg/m3";
    constant Real cp = 0 "specific heat, J/(kg.K)";
    function viscosity
      input Real T;
      output Real mu;
    algorithm
      mu := 0;
    end viscosity;
  end PartialMedium;

  package Water
    extends PartialMedium;
    constant Real rho = 1000.0;
    constant Real cp = 4186.0;
    function viscosity
      input Real T;
      output Real mu;
    algorithm
      mu := 0.001 * exp(-0.02 * (T - 20));
    end viscosity;
  end Water;

  package Oil
    extends PartialMedium;
    constant Real rho = 900.0;
    constant Real cp = 1900.0;
    function viscosity
      input Real T;
      output Real mu;
    algorithm
      mu := 0.1 * exp(-0.05 * (T - 20));
    end viscosity;
  end Oil;
end Media;

model HeatedTank "A tank whose fluid is a replaceable package"
  // The Fluid-style idiom: the component is written against an
  // interface package and any medium honouring it can be swapped in
  // from outside, constants and functions alike.
  replaceable package Medium = Media.Water constrainedby Media.PartialMedium;
  parameter Real volume = 0.2 "m3";
  parameter Real power = 50000.0 "W";
  Real T(start = 20.0, fixed = true) "temperature";
  Real mu "viscosity of whatever the medium is";
equation
  der(T) = power / (volume * Medium.rho * Medium.cp);
  mu = Medium.viscosity(T);
  annotation(experiment(StopTime = 600.0, Interval = 1.0));
end HeatedTank;

model OilTank "The same tank holding oil instead"
  extends HeatedTank(redeclare package Medium = Media.Oil);
  annotation(experiment(StopTime = 600.0, Interval = 1.0));
end OilTank;
