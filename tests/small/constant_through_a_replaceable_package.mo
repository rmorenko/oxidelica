// The fluid libraries reach a medium's constants through the package
// the model was given: `parameter h_start = Medium.h_default`, where
// `Medium` is a replaceable package redeclared at the site. Refused
// as `nothing gives a value to `Medium.h_default``, which is what
// twenty-five of the fluid examples die of.
model ConstantThroughAReplaceablePackage
  package Base
    constant Real p_default = 100000;
    constant Real T_default = 293.15;
    function enthalpy
      input Real p;
      input Real T;
      output Real h;
    algorithm
      h := 1000*T + p/1000;
    end enthalpy;
    constant Real h_default = enthalpy(p_default, T_default);
  end Base;
  package Air
    extends Base(T_default = 300);
  end Air;
  replaceable package Medium = Base;
  parameter Real h_start = Medium.h_default;
  Real x(start = h_start, fixed = true);
equation
  der(x) = 0;
  annotation(experiment(StopTime = 1, Interval = 1));
end ConstantThroughAReplaceablePackage;
