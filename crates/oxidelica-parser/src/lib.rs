//! oxidelica-parser — лексер, AST и парсер среза Modelica (веха M0).

pub mod ast;
pub mod lexer;
pub mod parser;

pub use ast::{BinOp, Component, EquationItem, Experiment, Expr, Model, Variability};
pub use parser::{parse_model, ParseError};
