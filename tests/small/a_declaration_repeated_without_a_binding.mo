// Squeezed down from
// Modelica.Electrical.QuasiStatic.SinglePhase.Examples.SeriesResonance.
//
// The same repetition as `a_declaration_repeated_from_a_base.mo`, but
// with no binding on either copy. `TwoPinElementary` declares
// `SI.AngularVelocity omega` and states it in an equation of its own;
// `TwoPin`, which extends it, declares the identical `omega` again and
// states nothing. A repeated declaration is one element and not two,
// binding or no binding - the language says an element handed down more
// than once is included once.
//
// Taken as two, the flat model carries two unknowns called `w` where it
// has one equation for them, and every quasi-static component of the
// electrical library is one unknown over. Seventeen of the nineteen
// names `EddyCurrentLosses` could not determine were this `omega`.
//
// The bindingless copy is the whole point: the rule that caught the
// bound repetition asked for a binding to be present, so this shape
// walked straight past it.
model Base
  Real w;
  Real g;
equation
  w = der(g);
end Base;

model ADeclarationRepeatedWithoutABinding
  extends Base;
  Real w;
equation
  g = 2.0 * time;
  annotation(experiment(StopTime = 1));
end ADeclarationRepeatedWithoutABinding;
