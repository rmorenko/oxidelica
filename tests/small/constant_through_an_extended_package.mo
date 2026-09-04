// A medium reached through a chain of extending packages, which is
// how the fluid libraries are built: `Air_ph extends Air_Base(...)`
// extends `PartialMedium`, and the model asks it for `h_default` - a
// constant whose value is a call on the package's own function.
model ConstantThroughAnExtendedPackage
  package Partial
    constant Real p_default = 100000;
    constant Real T_default = 293.15;
    replaceable function enthalpy
      input Real p;
      input Real T;
      output Real h;
    algorithm
      h := 1000*T + p/1000;
    end enthalpy;
    constant Real h_default = enthalpy(p_default, T_default);
  end Partial;
  package AirBase
    extends Partial;
  end AirBase;
  package AirPh
    extends AirBase(T_default = 300);
  end AirPh;
  replaceable package Medium = Partial;
  parameter Real h_start = Medium.h_default;
  Real x(start = h_start, fixed = true);
equation
  der(x) = 0;
  annotation(experiment(StopTime = 1, Interval = 1));
end ConstantThroughAnExtendedPackage;
