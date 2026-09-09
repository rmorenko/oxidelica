// Squeezed from Modelica.Electrical.Machines.Examples.InductionMachines.IMC_DOL
// to a machine and a load, and standing for twenty-one machine models
// refused for `cannot differentiate through algebraic variable`.
//
// A rigid connection is one degree of freedom written twice, so index
// reduction demotes one of the angles and gives it a dummy derivative.
// The names beside it are defined by plain equalities with that demoted
// angle - `inertiaRotor.flange_b.phi = inertiaRotor.phi` - and the
// derivative of a demoted state is exactly what the dummy is.
//
// The fixpoint that gathers definitions did not know that. It accepted
// a definition only when every name in it was a parameter, a state or
// already accepted, and a demoted state is none of the three, so the
// definition was thrown away and a flange whose angle is a rotor's was
// called a variable nothing can differentiate.
//
// Supplying that definition was built and measured, and it is parked.
// It takes this model and IMC_DOL to the next wall - `aimc.fixed.
// flange.phi = aimc.airGap.support.phi` constrains no state - and the
// transformer shape with them.
//
// One cause of the unbounded growth behind it has since been removed
// and shipped on its own: the quotient rule squared a denominator that
// does not move, and index reduction differentiates its own output, so
// `J^2` became `(J^2)^2` once per reduction.
// ControlledDCDrives.CurrentControlledDCPM no longer dies of memory -
// it reaches a verdict - but it takes 338 seconds to do it, and a
// corpus run under the rule is still killed on another model. What
// remains is that a definition is inlined wherever its name appears,
// with no sharing, so the twentieth reduction still builds an
// expression of ninety million characters. That is the next wall, and
// it is a different one from the wall this model names.
model DerivativeThroughADemotedState
  Modelica.Electrical.Machines.BasicMachines.InductionMachines.IM_SquirrelCage aimc;
  Modelica.Mechanics.Rotational.Components.Inertia load(J = 0.29);
equation
  connect(aimc.flange, load.flange_a);
  annotation(experiment(StopTime = 1));
end DerivativeThroughADemotedState;
