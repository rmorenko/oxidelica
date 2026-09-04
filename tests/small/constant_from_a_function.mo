// A constant whose value is a call: the medium libraries write
// `constant SpecificEnthalpy h_default = specificEnthalpy_pTX(...)`
// and a model starts its states from it. Refused as
// `cannot evaluate parameters [...]`, which is what twenty-five of
// the fluid examples die of.
model ConstantFromAFunction
  package M
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
  end M;
  parameter Real h_start = M.h_default;
  Real x(start = h_start, fixed = true);
equation
  der(x) = 0;
  annotation(experiment(StopTime = 1, Interval = 1));
end ConstantFromAFunction;
