//! oxidelica-parser — lexer, AST and parser for the M0 Modelica slice.

#![deny(missing_docs)]

pub mod ast;
pub mod check;
pub mod flatten;
pub mod lexer;
pub mod library;
pub mod outside;
pub mod parser;
mod units;

pub use ast::{
    operator_name, BinOp, Causality, ClassDef, ClassKind, Component, EquationItem, Experiment,
    Expr, Model, RelOp, Statement, StatementBranch, Variability, WhenAction, WhenBranch,
    WhenClause,
};
pub use flatten::{
    class_info, flatten as flatten_named, name_counts, read_table_file, ClassInfo, Trail,
};
pub use library::{
    download_root, downloaded_libraries, library_directories, library_directory, library_files,
    library_files_in, library_sources, LIBRARY_VARIABLE, MODELICA_PATH,
};
pub use parser::{
    parse_file, parse_model, parse_model_reading, parse_model_with_libraries, ParseError,
};
