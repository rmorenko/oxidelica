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
// transformer shape with them, but it also makes
// ControlledDCDrives.CurrentControlledDCPM grow without bound and be
// killed, so no corpus number could be taken. A definition reached
// through a dummy is a definition reached through the equation that
// determined the dummy, and somewhere that walk stops being finite.
model DerivativeThroughADemotedState
  Modelica.Electrical.Machines.BasicMachines.InductionMachines.IM_SquirrelCage aimc;
  Modelica.Mechanics.Rotational.Components.Inertia load(J = 0.29);
equation
  connect(aimc.flange, load.flange_a);
  annotation(experiment(StopTime = 1));
end DerivativeThroughADemotedState;
