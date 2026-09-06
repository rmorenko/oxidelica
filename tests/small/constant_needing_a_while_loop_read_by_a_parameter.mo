// A package constant whose binding is a call the arithmetic round
// cannot fold - `h_default = dsolve(target)`, where `dsolve` iterates
// with a `while` - read by a parameter of a model.
//
// The while loop is decidable: its trip count depends only on the
// constant `target`, so the compiler can run it at translation. But
// the call reached the fold with its argument still a bare name:
// `target` is a sibling constant of the same package, and the road
// that substitutes a constant's binding resolves the call under the
// package's scope, where a bare `target` is not looked up as one of
// the package's own constants. Left unfolded, the call reached the
// flat model as `dsolve(target)` with an argument nothing out there
// declares, and the parameter reading it was refused as `cannot
// evaluate parameters [...]: nothing gives a value`.
//
// The fix folds the package's already-settled constants into each
// binding before it is walked, so `dsolve(target)` becomes
// `dsolve(16)` and the loop runs to a number at translation.
//
// This is the foldable cousin of the corpus media constant. The head
// of the queue, ReferenceAir.DryAir1, needs this AND a separate fix:
// its `h_default` fails earlier, where `setState_pTX` answers with an
// empty `ThermodynamicState` because the medium's redeclaration of
// that record did not reach the inherited body that builds it. That
// record-resolution defect is not this one, and the corpus victim is
// untouched until both are cleared. What this fixes is the shape
// where the only thing missing was the sibling argument.
package P
  function dsolve
    input Real target;
    output Real x;
  protected
    Integer i = 0;
    Boolean found = false;
    Real f;
  algorithm
    x := 1.0;
    while ((i < 100) and not found) loop
      f := x*x - target;
      if abs(f) <= 1e-9 then
        found := true;
      end if;
      x := x - f/(2*x);
      i := i + 1;
    end while;
  end dsolve;
  package Medium
    constant Real target = 16.0;
    constant Real h_default = dsolve(target);
  end Medium;
  model Use
    parameter Real h_start = Medium.h_default;
    Real z(start = h_start, fixed = true);
  equation
    der(z) = 0;
    annotation(experiment(StopTime = 1));
  end Use;
end P;
