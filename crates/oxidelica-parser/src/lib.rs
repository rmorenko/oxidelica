//! oxidelica-parser — lexer, AST and parser for the M0 Modelica slice.

#![deny(missing_docs)]

pub mod ast;
pub mod lexer;
pub mod parser;

pub use ast::{BinOp, Component, EquationItem, Experiment, Expr, Model, Variability};
pub use parser::{parse_model, ParseError};
