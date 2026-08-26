//! The example models in `examples/`, run and checked against what
//! they are meant to do.
//!
//! These are whole models rather than rules: each is a thing someone
//! would actually simulate, and what it is checked against is what
//! theory, or a closed form, or arithmetic says it should come to.
//!
//! One file per kind of model, so that a new example knows where it
//! goes without anybody having to say.

mod shared;

mod arrays;
mod clocked;
mod events;
mod language;
mod library;
mod structure;
