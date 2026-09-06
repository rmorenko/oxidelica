// A medium's `constant h_default = enthalpy_pT(p, T)` where the body
// of `enthalpy_pT` is written in the partial base and calls a
// `replaceable partial` sibling - `enthalpy(setState(p, T))` - that
// only the package extending the base redeclares.
//
// The wrapper call is bare: no head names the package, so nothing at
// the fold said under which class the body's own names were to be
// read. The bare `enthalpy`/`setState` inside resolved in the base,
// where they are still partial, and the redeclared bodies of the
// medium the model actually named were never reached. The constant
// stayed a bare name and every parameter reading it was refused as
// `cannot evaluate parameters [...]: nothing gives a value`. The fix
// holds the scope the constant was asked from across the wrapper
// body, so the redeclared siblings are found.
//
// This is the *foldable* cousin of the corpus media failure. The head
// of the queue, ReferenceAir.DryAir1, is a deeper case: its
// `h_default` bottoms out in a Newton iteration (`Inverses.dofpT`, a
// `while` loop), which no translate-time fold can evaluate, and it is
// untouched by this. What this fixes is the shape where the redeclared
// bodies do fold once they are reached at all.
package P
  partial package PartialMedium
    constant Real p_default = 100000;
    constant Real T_default = 293.15;
    record State
      Real p;
      Real T;
      Real h;
    end State;
    replaceable partial function setState
      input Real p; input Real T; output State s;
    end setState;
    replaceable partial function enthalpy
      input State s; output Real h;
    end enthalpy;
    function enthalpy_pT
      input Real p; input Real T; output Real h;
    algorithm
      h := enthalpy(setState(p, T));
    end enthalpy_pT;
    constant Real h_default = enthalpy_pT(p_default, T_default);
  end PartialMedium;
  package Air
    extends PartialMedium;
    redeclare function extends setState
    algorithm
      s := State(p = p, T = T, h = 1000*T + p/1000);
    end setState;
    redeclare function extends enthalpy
    algorithm
      h := s.h;
    end enthalpy;
  end Air;
  model DryAir1
    replaceable package Medium = PartialMedium;
    parameter Real h_start = Medium.h_default;
    Real x(start = h_start, fixed = true);
  equation
    der(x) = 0;
    annotation(experiment(StopTime = 1));
  end DryAir1;
  model M
    DryAir1 d(redeclare package Medium = Air);
  end M;
end P;
