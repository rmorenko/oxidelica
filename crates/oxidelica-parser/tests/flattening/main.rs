//! Flattening, asked of the crate from outside: a source string in, a
//! flat model out.
//!
//! These are the language's own rules, checked through `parse_model`,
//! which is how everything but the compiler itself sees this crate.
//! One file per thing the rules are about, so that a new test knows
//! where it goes without anybody having to say.

mod shared;

mod algorithms;
mod arrays;
mod clocks;
mod connections;
mod inheritance;
mod names;
mod records;
mod tables;
