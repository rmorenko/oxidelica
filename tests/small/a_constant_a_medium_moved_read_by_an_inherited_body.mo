// Squeezed down from ModelicaTest.Media.TestAllProperties.LinearColdWater.
//
// The interface gives `reference_T` the value 298.15, and the cold water
// that extends it through the linear fluid moves it to 278.15. The body
// of `density`, written in the middle package, read the interface's
// number: the walk outwards asked the medium on the mark, the mark
// declined because the constant carries a unit, and the walk went on to
// fold the interface's own digit. At the reference temperature the
// density came out 1002.18 rather than 997.05, and nothing refused.
package AConstantAMediumMovedReadByAnInheritedBody
  partial package Base
    constant Real reference_p(unit="Pa") = 101325;
    constant Real reference_T(unit="K") = 298.15;
    replaceable record ThermodynamicState end ThermodynamicState;
    replaceable partial function setState_pT
      input Real p(unit="Pa"); input Real T(unit="K");
      output ThermodynamicState state;
    end setState_pT;
    replaceable partial function density
      input ThermodynamicState state;
      output Real d(unit="kg/m3");
    end density;
    replaceable function density_pT
      input Real p(unit="Pa"); input Real T(unit="K");
      output Real d(unit="kg/m3");
    algorithm
      d := density(setState_pT(p, T));
    end density_pT;
  end Base;
  partial package Linear
    extends Base;
    constant Real beta(unit="1/K");
    constant Real kappa(unit="1/Pa");
    constant Real reference_d(unit="kg/m3");
    redeclare record ThermodynamicState
      Real p(unit="Pa"); Real T(unit="K");
    end ThermodynamicState;
    redeclare function extends setState_pT
    algorithm
      state := ThermodynamicState(p=p, T=T);
    end setState_pT;
    redeclare function extends density
    algorithm
      d := (1 + (state.p - reference_p)*kappa - (state.T - reference_T)*beta)*reference_d;
    end density;
  end Linear;
  package Cold
    extends Linear(reference_T=278.15, beta=2.5713e-4, kappa=4.5154e-10, reference_d=997.05);
  end Cold;
  partial model M
    replaceable package Medium = Base;
    Real d2(unit="kg/m3") = Medium.density_pT(Medium.reference_p, Medium.reference_T);
  end M;
  model Test
    extends M(redeclare package Medium = Cold);
    annotation(experiment(StopTime = 0.01));
  end Test;
end AConstantAMediumMovedReadByAnInheritedBody;
