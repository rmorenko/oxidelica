//! oxidelica-parser — lexer, AST and parser for the M0 Modelica slice.

#![deny(missing_docs)]

pub mod ast;
pub mod flatten;
pub mod lexer;
pub mod library;
pub mod parser;

pub use ast::{
    BinOp, ClassDef, ClassKind, Component, EquationItem, Experiment, Expr, Model, RelOp, Statement,
    StatementBranch, Variability, WhenAction, WhenBranch, WhenClause,
};
pub use flatten::{class_info, ClassInfo};
pub use library::{library_directory, library_sources, LIBRARY_VARIABLE};
pub use parser::{parse_file, parse_model, parse_model_with_libraries, ParseError};
