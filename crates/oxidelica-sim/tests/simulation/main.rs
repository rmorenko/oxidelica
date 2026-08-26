//! Simulation, asked of the crate from outside: a source string in, a
//! result out.
//!
//! One file per thing a run can be about, so that a new test knows
//! where it goes without anybody having to say.

mod shared;

mod equations;
mod events;
mod functions;
mod initialisation;
mod output;
mod refusals;
mod solvers;
mod systems;
