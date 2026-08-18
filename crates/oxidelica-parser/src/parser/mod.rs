//! Recursive-descent parser for the M0 Modelica slice.
//!
//! Slice grammar:
//! ```text
//! model      : "model" IDENT [STRING] { declaration } ["equation" { eq_item }]
//!              ["annotation" "(" ... ")" ";"] "end" IDENT ";"
//! declaration: ["parameter"|"constant"] "Real" IDENT ["(" attr {"," attr} ")"]
//!              ["=" expr] [STRING] ";"
//! attr       : IDENT "=" (expr | "true" | "false")
//! eq_item    : expr "=" expr [STRING] ";"
//! when_eq    : "when" expr "then" { action ";" } "end" "when" ";"
//! action     : "reinit" "(" IDENT "," expr ")" | "terminate" "(" STRING ")"
//! expr       : if_expr | logical_or
//! if_expr    : "if" expr "then" expr {"elseif" expr "then" expr} "else" expr
//! logical_or : logical_and { "or" logical_and }
//! logical_and: logical_not { "and" logical_not }
//! logical_not: ["not"] relation
//! relation   : arith [("<"|"<="|">"|">="|"=="|"<>") arith]
//! arith      : ["+"|"-"] term { ("+"|"-") term }
//! term       : factor { ("*"|"/") factor }
//! factor     : primary ["^" primary]
//! primary    : NUMBER | "true" | "false" | IDENT ["(" [expr {"," expr}] ")"]
//!            | "(" expr ")" | "time"
//! ```
//! Precedence follows the Modelica specification: `^` binds tighter than
//! `*`/`/`; unary minus lives at the addition level (`-a*b` = `-(a*b)`).

use crate::ast::*;
use crate::lexer::{lex, Spanned, Token};
use std::fmt;

/// A parse error with its source line.
#[derive(Debug)]
pub struct ParseError {
    /// Human-readable description of the problem.
    pub message: String,
    /// 1-based line number where the error occurred.
    pub line: u32,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parse all class definitions from a Modelica source file.
pub fn parse_file(source: &str) -> Result<Vec<ClassDef>, ParseError> {
    parse_file_within(source).map(|(classes, _)| classes)
}

/// As [`parse_file`], and also the namespace the file declared itself
/// to sit in, when it said.
fn parse_file_within(source: &str) -> Result<(Vec<ClassDef>, Option<String>), ParseError> {
    let tokens = lex(source).map_err(|e| ParseError {
        message: e.message,
        line: e.line,
    })?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        in_subscript: false,
    };
    let mut classes = Vec::new();
    let mut within: Option<String> = None;
    while parser.peek() != &Token::Eof {
        if parser.peek() == &Token::Within {
            parser.bump();
            // `within A.B;` says where in the tree this file sits, so
            // its classes are known by their place in it - which is
            // what lets a library be spread over a directory.
            if matches!(parser.peek(), Token::Ident(_)) {
                within = Some(parser.dotted_name("within namespace")?);
            }
            parser.expect(&Token::Semi, "semicolon after within")?;
            continue;
        }
        let class = match parser.class_def()? {
            ClassItem::Class(class) => *class,
            ClassItem::Alias(alias) => {
                return Err(ParseError {
                    message: format!(
                        "`{}` is a class alias; the top level of a file needs a full definition",
                        alias.name
                    ),
                    line: 1,
                })
            }
        };
        // A package contributes its members to the registry under
        // qualified names, alongside the package itself.
        let mut class = class;
        if let Some(namespace) = &within {
            class.name = format!("{namespace}.{}", class.name);
        }
        flatten_packages(class, &mut classes);
    }
    if classes.is_empty() {
        return Err(ParseError {
            message: "no class definitions in file".into(),
            line: 1,
        });
    }
    Ok((classes, within))
}

/// Collect a class and everything nested inside it, qualifying the
/// names of package members.
fn flatten_packages(mut class: ClassDef, out: &mut Vec<ClassDef>) {
    let nested = std::mem::take(&mut class.nested);
    let prefix = class.name.clone();
    out.push(class);
    for mut inner in nested {
        inner.name = format!("{prefix}.{}", inner.name);
        flatten_packages(inner, out);
    }
}

/// Parse a source file and flatten its last `model` class into a flat
/// [`Model`] ready for compilation.
pub fn parse_model(source: &str) -> Result<Model, ParseError> {
    parse_model_with_libraries(&[], source)
}

/// Parse a model in the context of library sources: the libraries only
/// contribute class definitions, the top-level model comes from
/// `source`.
pub fn parse_model_with_libraries(libraries: &[String], source: &str) -> Result<Model, ParseError> {
    let mut classes = Vec::new();
    for library in libraries {
        classes.extend(parse_file(library)?);
    }
    let (own, within) = parse_file_within(source)?;
    // The model to simulate is the last one written at the top level of
    // the file; models nested inside other classes are components' types,
    // not entry points. A file that says where it sits carries that
    // much of every name, so the test for "top level" looks past it.
    let namespace = within.map(|space| format!("{space}.")).unwrap_or_default();
    let top_level = |class: &ClassDef| {
        class
            .name
            .strip_prefix(&namespace)
            .is_some_and(|rest| !rest.contains('.'))
    };
    let top = own
        .iter()
        .rev()
        .find(|c| c.kind == ClassKind::Model && !c.partial && top_level(c))
        .or_else(|| {
            own.iter()
                .rev()
                .find(|c| c.kind == ClassKind::Model && !c.partial)
        })
        .ok_or_else(|| ParseError {
            message: "no model class in file".into(),
            line: 1,
        })?
        .name
        .clone();
    classes.extend(own);
    crate::flatten::flatten(&classes, &top).map_err(|message| ParseError { message, line: 0 })
}

struct Parser {
    tokens: Vec<Spanned>,
    pos: usize,
    /// Whether a subscript is being parsed: only there does the `end`
    /// keyword mean "the length of this dimension".
    in_subscript: bool,
}

/// What a modifier list contributes: value modifiers by (possibly
/// dotted) name, plus the redeclarations found among them.
type Modifications = (Vec<(String, Expr)>, Vec<Redeclare>);
/// Attribute defaults of a `type` alias, with the `unit` string apart.
type AliasAttributes = (Vec<(String, Expr)>, Option<String>);

/// One item of a class body that introduces a class: a full definition
/// or a short alias.
enum ClassItem {
    /// An ordinary class definition, boxed to keep the enum small.
    Class(Box<ClassDef>),
    /// A short one: `package Medium = Media.Water;`.
    Alias(ClassAlias),
}

/// The contents of one branch of an `if` equation.
type BranchBody = (Vec<EquationItem>, Vec<(Expr, Expr)>);

mod classes;
mod declarations;
mod equations;
mod expressions;
mod statements;

impl Parser {
    pub(super) fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    /// The token `offset` places past the current one; the lexer ends
    /// the stream with `Eof`, which stands in past the end too.
    pub(super) fn peek_at(&self, offset: usize) -> &Token {
        self.tokens
            .get(self.pos + offset)
            .map_or(&Token::Eof, |spanned| &spanned.token)
    }

    /// The token `ahead` positions past the current one, clamped to the
    /// end-of-file marker. Used where a prefix keyword alone does not
    /// say whether a class or a component follows.
    pub(super) fn peek_ahead(&self, ahead: usize) -> &Token {
        let index = (self.pos + ahead).min(self.tokens.len() - 1);
        &self.tokens[index].token
    }

    pub(super) fn line(&self) -> u32 {
        self.tokens[self.pos].line
    }

    pub(super) fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].token.clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    pub(super) fn expect(&mut self, expected: &Token, context: &str) -> Result<(), ParseError> {
        if self.peek() == expected {
            self.bump();
            Ok(())
        } else {
            Err(self.err(format!(
                "expected `{expected}` ({context}), found `{}`",
                self.peek()
            )))
        }
    }

    pub(super) fn err(&self, message: String) -> ParseError {
        ParseError {
            message,
            line: self.line(),
        }
    }

    pub(super) fn ident(&mut self, context: &str) -> Result<String, ParseError> {
        match self.peek().clone() {
            Token::Ident(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(self.err(format!("expected identifier ({context}), found `{other}`"))),
        }
    }

    pub(super) fn opt_string(&mut self) -> Option<String> {
        if let Token::Str(s) = self.peek().clone() {
            self.bump();
            Some(s)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every one of these should be refused, and say why.
    fn refused(source: &str) -> String {
        parse_model(source).expect_err("should be refused").message
    }

    #[test]
    fn a_file_says_what_is_wrong_with_it() {
        // The lexer's own complaints carry through with their line.
        let broken = parse_model("model M Real x @ 1; end M;").unwrap_err();
        assert!(broken.line >= 1, "{broken:?}");
        // A file with nothing in it at all.
        assert!(parse_file("// just a comment\n")
            .unwrap_err()
            .message
            .contains("no class definitions"));
        assert!(refused("package P end P;").contains("no model class"));
        assert!(refused("type T = Real;").contains("no model class"));
    }

    #[test]
    fn class_headers_are_refused_when_they_do_not_make_sense() {
        assert!(refused("equation x = 1;").contains("expected a class definition"));
        assert!(refused("model M Real y; equation assert(y > 0, 1); end M;")
            .contains("assert expects a string message"));
        assert!(refused("model M import Lib.{A B}; end M;").contains("in an import list"));
        // Annotations are accepted, and skipped, wherever they may sit.
        let m = parse_model(
            "model M import Lib.A; parameter Real k = 1 annotation(y = 2); Real y; equation y = k annotation(z = 3); assert(y > -1, \"fine\", AssertionLevel.warning) annotation(w = 4); end M;",
        )
        .unwrap();
        assert_eq!(m.equations.len(), 1);
        // A type alias may say `false` for an attribute.
        let m = parse_model(
            "type T = Real(fixed = false, unit = \"m\"); model M T x(start = 1); Real y; equation y = x; end M;",
        )
        .unwrap();
        assert_eq!(m.components[0].fixed, Some(false));
    }

    #[test]
    fn declarations_are_refused_when_they_do_not_make_sense() {
        assert!(refused("model M Real x(unit = 3); end M;").contains("unit expects a string"));
        assert!(refused("model M Real x(start = 1 fixed = true); end M;")
            .contains("expected `,` or `)` in attributes"));
        assert!(
            refused("model Sub Real k; end Sub; model M Sub s(k = 1 j = 2); end M;")
                .contains("expected `,` or `)` in modifiers")
        );
        // `final` and `each` are accepted and pass without meaning.
        let m = parse_model(
            "model Sub parameter Real k[2] = {1, 2}; end Sub; model M final Sub s(each k = 3); end M;",
        )
        .unwrap();
        assert!(m.components.iter().any(|c| c.name == "s.k[2]"));
        // A modifier with no value at all is allowed and says nothing.
        let m = parse_model(
            "model Sub parameter Real k = 1; Real y; equation y = k; end Sub; model M Sub s(k); Real out; equation out = s.y; end M;",
        )
        .unwrap();
        assert!(m.components.iter().any(|c| c.name == "s.k"));
    }

    #[test]
    fn equations_are_refused_when_they_do_not_make_sense() {
        assert!(
            refused("model M Real y; equation for i loop y = 1; end for; end M;")
                .contains("in after the loop variable")
        );
        assert!(
            refused("model M Real y; equation for i in 3 loop y = 1; end for; end M;")
                .contains("a loop needs a range")
        );
        assert!(refused("model M Real y; equation for i in 1:2 loop y = 1;")
            .contains("unterminated for equation"));
        assert!(
            refused("model M Real y; equation for i in 1:2 loop end for; y = 1; end M;")
                .contains("for equation has no body")
        );
        assert!(refused("model M Real v[2]; equation v[1 2] = 0; end M;")
            .contains("expected `,` or `]` in a subscript"));
        assert!(
            refused("model M Real y; equation Connections.knot(y); end M;")
                .contains("is not a clause this compiler knows")
        );
        // A subscripted reference may be followed by a member, and a
        // matrix row must close properly.
        let m = parse_model(
            "record P Real x; end P; model M P points[2]; Real y; equation points[1].x = 1; points[2].x = 2; y = points[2].x; end M;",
        )
        .unwrap();
        assert!(m.components.iter().any(|c| c.name == "points[2].x"));
        assert!(refused("model M Real y; equation y = [1, 2 3]; end M;")
            .contains("expected `,`, `;` or `]` in a matrix"));
    }

    #[test]
    fn statements_are_refused_when_they_do_not_make_sense() {
        assert!(refused(
            "model M Real v[2]; Real y; equation v = {1, 2}; algorithm y := v[1 2]; end M;"
        )
        .contains("expected `,` or `]` in a subscript"));
        assert!(
            refused("model M Real a; Real b; algorithm (a b) := 1; end M;")
                .contains("expected `,` or `)` in a tuple of targets")
        );
        assert!(
            refused("model M Real y; algorithm for i loop y := 1; end for; end M;")
                .contains("in after the loop variable")
        );
        assert!(
            refused("model M Real y; algorithm for i in 3 loop y := 1; end for; end M;")
                .contains("a loop needs a range")
        );
        assert!(
            refused("model M Real y; algorithm for i in 1:2:9 loop y := 1; end for; end M;")
                .contains("a loop range with a step")
        );
        // Annotations may follow an assignment or a tuple assignment.
        let m = parse_model(
            "function two output Real a; output Real b; algorithm a := 1; b := 2; end two; model M Real p; Real q; Real r; algorithm r := 3 annotation(x = 1); (p, q) := two() annotation(y = 2); end M;",
        )
        .unwrap();
        assert_eq!(m.equations.len(), 3);
        // A `when` among the statements may watch several conditions.
        let m = parse_model(
            "model M Real u; discrete Real c(start = 0); equation u = time; algorithm when {u > 0.3, u > 0.6} then c := pre(c) + 1; end when; end M;",
        )
        .unwrap();
        assert_eq!(m.when_clauses[0].branches.len(), 2);
    }

    #[test]
    fn the_elementwise_operators_all_parse() {
        let m = parse_model(
            "model M Real a[2]; Real b[2]; Real s[2]; Real d[2]; Real p[2]; Real q[2]; Real e[2]; equation a = {1, 2}; b = {3, 4}; s = a .+ b; d = a .- b; p = a .* b; q = a ./ b; e = a .^ b; end M;",
        )
        .unwrap();
        // Two elements apiece for seven whole-array equations.
        assert_eq!(m.equations.len(), 14);
    }

    #[test]
    fn parses_minimal_model() {
        let m = parse_model("model M Real x(start=1); equation der(x) = -x; end M;").unwrap();
        assert_eq!(m.name, "M");
        assert_eq!(m.components.len(), 1);
        assert_eq!(m.equations.len(), 1);
        assert_eq!(m.equations[0].lhs.as_der_of(), Some("x"));
    }

    #[test]
    fn respects_precedence() {
        // -a*b^2 == -((a)*(b^2))
        let m = parse_model("model M Real a; Real b; Real c; equation c = -a*b^2; end M;").unwrap();
        let rhs = &m.equations[0].rhs;
        let Expr::Neg(inner) = rhs else {
            panic!("expected unary minus: {rhs:?}")
        };
        let Expr::Bin(BinOp::Mul, _, r) = inner.as_ref() else {
            panic!("expected multiplication: {inner:?}")
        };
        assert!(matches!(r.as_ref(), Expr::Bin(BinOp::Pow, _, _)));
        // Unary plus is transparent.
        let m2 = parse_model("model M Real c; equation c = +5; end M;").unwrap();
        assert_eq!(m2.equations[0].rhs, Expr::Number(5.0));
    }

    #[test]
    fn extracts_experiment() {
        let m = parse_model(
            "model M Real x; equation x = time; \
             annotation(uses(Modelica(version=\"4.0.0\")), experiment(StopTime=5.0, Interval=0.01)); \
             end M;",
        )
        .unwrap();
        assert_eq!(m.experiment.stop_time, Some(5.0));
        assert_eq!(m.experiment.interval, Some(0.01));
    }

    #[test]
    fn rejects_mismatched_end() {
        assert!(parse_model("model A Real x; equation x = 1; end B;").is_err());
    }

    fn err_of(source: &str) -> String {
        parse_model(source).unwrap_err().to_string()
    }

    #[test]
    fn error_paths_are_reported() {
        // End of file without `end`.
        assert!(err_of("model M Real x;").contains("end of file"));
        // Unknown component type at flattening.
        assert!(err_of("model M Widget x; end M;").contains("unknown type"));
        // A descriptive attribute the compiler has no use for is
        // dropped, not rejected: the standard library sets these on
        // most variables. `min` and `max` are kept, and checked.
        let tolerated = parse_model(
            "model M Real x(min = 0, max = 5, nominal = 3, unit = \"m\", start = 2); end M;",
        )
        .unwrap();
        assert_eq!(tolerated.components[0].start, Some(Expr::Number(2.0)));
        assert_eq!(tolerated.components[0].min, Some(Expr::Number(0.0)));
        assert_eq!(tolerated.components[0].max, Some(Expr::Number(5.0)));
        // Non-boolean fixed.
        assert!(err_of("model M Real x(fixed=1); end M;").contains("true/false"));
        // Missing comma between attributes.
        assert!(err_of("model M Real x(start=1 fixed=true); end M;").contains("`,` or `)`"));
        // Missing semicolon after an equation.
        assert!(err_of("model M Real x; equation x = 1 end M;").contains("semicolon"));
        // If without else.
        assert!(err_of("model M Real y; equation y = if 1 > 0 then 1; end M;").contains("else"));
        // Garbage in an expression.
        assert!(err_of("model M Real x; equation x = ;; end M;").contains("unexpected token"));
        // Unclosed parenthesis.
        assert!(err_of("model M Real x; equation x = (1 + 2; end M;").contains("closing"));
        // Model name is not an identifier.
        assert!(err_of("model 1 end 1;").contains("identifier"));
    }

    #[test]
    fn annotation_error_paths() {
        // Unterminated annotation (cut off while balancing parentheses).
        assert!(err_of("model M Real x; equation x = 1; annotation(uses(")
            .contains("unterminated annotation"));
        // Cut off inside experiment has its own diagnostics.
        assert!(
            err_of("model M Real x; equation x = 1; annotation(experiment(")
                .contains("experiment parameter")
        );
        // StartTime != 0 is unsupported.
        assert!(err_of(
            "model M Real x; equation x = 1; annotation(experiment(StartTime=2)); end M;"
        )
        .contains("StartTime"));
        // Not a number in experiment.
        assert!(err_of(
            "model M Real x; equation x = 1; annotation(experiment(StopTime=abc)); end M;"
        )
        .contains("expected a number"));
        // Garbage instead of `,` or `)`.
        assert!(err_of(
            "model M Real x; equation x = 1; annotation(experiment(StopTime=1 abc)); end M;"
        )
        .contains("`,` or `)` in experiment"));
    }

    #[test]
    fn annotation_accepts_start_time_zero_and_negatives() {
        let m = parse_model(
            "model M Real x; equation x = 1; \
             annotation(experiment(StartTime=0, StopTime=2, Tolerance=1e-6, Whatever=3)); end M;",
        )
        .unwrap();
        assert_eq!(m.experiment.stop_time, Some(2.0));
        assert_eq!(m.experiment.tolerance, Some(1e-6));
        // A negative literal goes through number_literal.
        let m2 = parse_model(
            "model M Real x; equation x = 1; annotation(experiment(StopTime=-1)); end M;",
        )
        .unwrap();
        assert_eq!(m2.experiment.stop_time, Some(-1.0));
    }

    #[test]
    fn parses_descriptions_and_der_on_rhs() {
        let m = parse_model(
            "model M \"a model\" Real x \"a variable\"; equation -x = der(x) \"an equation\"; end M;",
        )
        .unwrap();
        assert_eq!(m.description.as_deref(), Some("a model"));
        assert_eq!(m.components[0].description.as_deref(), Some("a variable"));
        assert_eq!(m.equations[0].rhs.as_der_of(), Some("x"));
    }

    #[test]
    fn parses_calls_booleans_and_constants() {
        let m = parse_model(
            "model M constant Real c = 2; parameter Real p = c + 1; Real y; \
             equation y = max(sin(time), 0) + (if true then p else 0); end M;",
        )
        .unwrap();
        assert_eq!(m.components[0].variability, Variability::Constant);
        assert_eq!(m.components[1].variability, Variability::Parameter);
        let mut refs = Vec::new();
        m.equations[0].rhs.collect_refs(&mut refs);
        assert_eq!(refs, vec!["p"]);
    }

    #[test]
    fn parses_if_elseif_chain() {
        let m = parse_model(
            "model M Real x; Real y; equation \
             y = if x > 1 then 1 elseif x < -1 then -1 else x; end M;",
        )
        .unwrap();
        // elseif unfolds into a nested If in the else branch.
        let Expr::If(_, _, else_branch) = &m.equations[0].rhs else {
            panic!("expected If: {:?}", m.equations[0].rhs)
        };
        assert!(matches!(else_branch.as_ref(), Expr::If(_, _, _)));
        // fixed=false parses as well.
        let m2 =
            parse_model("model M Real x(start=0, fixed=false); equation der(x)=1; end M;").unwrap();
        assert_eq!(m2.components[0].fixed, Some(false));
    }

    #[test]
    fn parses_when_clauses() {
        let m = parse_model(
            "model M Real x; equation x = time; \
             when x > 1 and time > 0.5 then terminate(\"done\"); end when; end M;",
        )
        .unwrap();
        assert_eq!(m.when_clauses.len(), 1);
        assert!(matches!(
            m.when_clauses[0].branches[0].actions.as_slice(),
            [WhenAction::Terminate(message)] if message == "done"
        ));
        // reinit is the other supported action, and clauses may hold
        // several of them.
        let with_reinit = parse_model(
            "model M Real x(start = 1); equation der(x) = -1; \
             when x < 0 then reinit(x, 1); terminate(\"bounced\"); end when; end M;",
        )
        .unwrap();
        assert_eq!(with_reinit.when_clauses[0].branches[0].actions.len(), 2);
        assert!(matches!(
            with_reinit.when_clauses[0].branches[0].actions[0],
            WhenAction::Reinit(ref name, _) if name == "x"
        ));
        // Errors: an unsupported action, an empty body, an unterminated
        // clause and a non-string message.
        assert!(
            err_of("model M Real x; equation x = 1; when x > 1 then end when; end M;")
                .contains("no actions")
        );
        assert!(
            err_of("model M Real x; equation x = 1; when x > 1 then reinit(x, 0);")
                .contains("unterminated when")
        );
        assert!(err_of(
            "model M Real x; equation x = 1; when x > 1 then terminate(42); end when; end M;"
        )
        .contains("string message"));
    }

    #[test]
    fn parses_declarations_the_way_the_standard_library_writes_them() {
        // Prefixes in any order, an attribute list full of things the
        // compiler ignores, a graphical annotation on the declaration
        // itself, and `final`/`each` inside a modifier.
        let classes = parse_file(
            "package Lib \
               connector Pin Real v; flow Real i; end Pin; \
               partial model OnePort Pin p; Pin n; end OnePort; \
               model Resistor \"Ideal resistor\" \
                 extends OnePort; \
                 inner replaceable parameter Real R(final unit = \"Ohm\", min = 0, \
                   nominal = 100, start = 1) = 1 \"Resistance\" \
                   annotation (Dialog(group = \"Electrical\")); \
                 Real v(stateSelect = StateSelect.never) \
                   annotation (Placement(transformation(extent = {{-10, -10}, {10, 10}}))); \
               equation \
                 v = R * p.i annotation (Documentation(info = \"<html></html>\")); \
                 connect(p, n) annotation (Line(points = {{-90, 0}, {90, 0}})); \
               end Resistor; \
             end Lib;",
        )
        .unwrap();
        let resistor = classes.iter().find(|c| c.name == "Lib.Resistor").unwrap();
        let r = resistor.components.iter().find(|c| c.name == "R").unwrap();
        assert_eq!(r.variability, Variability::Parameter);
        assert_eq!(r.scope, Scope::Inner);
        assert!(r.replaceable);
        assert_eq!(r.start, Some(Expr::Number(1.0)));
        assert_eq!(resistor.equations.len(), 1);
        assert_eq!(resistor.connects.len(), 1);
    }

    #[test]
    fn parses_redeclarations_and_conditions() {
        let classes = parse_file(
            "package Lib \
               partial model SISO Real u; Real y; end SISO; \
               model Gain extends SISO; parameter Real k = 1; equation y = k * u; end Gain; \
               model Line \
                 parameter Boolean useLimiter = false; \
                 replaceable Gain block1(k = 2) constrainedby SISO \"the block\"; \
                 Gain limiter if useLimiter; \
               end Line; \
               model Tuned extends Line(redeclare replaceable Gain block1(final k = 4), \
                 useLimiter = true); \
               end Tuned; \
             end Lib;",
        )
        .unwrap();
        let line = classes.iter().find(|c| c.name == "Lib.Line").unwrap();
        let block = line.components.iter().find(|c| c.name == "block1").unwrap();
        assert!(block.replaceable);
        assert_eq!(block.constrained_by.as_deref(), Some("SISO"));
        let limiter = line
            .components
            .iter()
            .find(|c| c.name == "limiter")
            .unwrap();
        assert!(limiter.condition.is_some());

        let tuned = classes.iter().find(|c| c.name == "Lib.Tuned").unwrap();
        let redeclare = &tuned.extends[0].redeclares[0];
        assert_eq!(redeclare.name, "block1");
        assert_eq!(redeclare.type_name, "Gain");
        assert_eq!(
            redeclare.modifiers,
            vec![("k".to_string(), Expr::Number(4.0))]
        );
        assert_eq!(
            tuned.extends[0].modifiers,
            vec![("useLimiter".to_string(), Expr::Bool(true))]
        );
    }

    #[test]
    fn parses_enumerations_and_structural_errors() {
        let classes = parse_file(
            "package Types type Init = enumeration(NoInit \"as declared\", SteadyState) \
             \"how a block starts\"; end Types;",
        )
        .unwrap();
        let init = classes.iter().find(|c| c.name == "Types.Init").unwrap();
        assert_eq!(init.enumeration, vec!["NoInit", "SteadyState"]);

        // Error paths of the new syntax.
        assert!(
            err_of("model M Real x; equation if x > 0 then end if; end M;")
                .contains("no equations")
        );
        assert!(err_of("model M Real x; equation if x > 0 then x = 1;").contains("unterminated if"));
        assert!(
            err_of("package P type K = enumeration(A B); end P; model M Real x; end M;")
                .contains("`,` or `)` in an enumeration")
        );
        assert!(err_of(
            "model M Real x; Real y; equation y = 1; end M; \
             model N M m(x 2); end N;"
        )
        .contains("expected `=` or a nested modifier list"));
    }

    #[test]
    fn parses_the_remaining_standard_library_spellings() {
        // A type alias with an annotation, a nested class carrying the
        // `replaceable` prefix, an `extends` with an annotation, an
        // `inner outer` declaration and a nested redeclaration.
        let classes = parse_file(
            "package Lib \
               type Angle = Real(unit = \"rad\") annotation (Documentation(info = \"\")); \
               partial model SISO Real u; Real y; end SISO; \
               replaceable model Gain extends SISO; parameter Real k = 1; \
                 equation y = k * u; end Gain; \
               model World parameter Real g = 1; end World; \
               model Line extends SISO annotation (Icon()); \
                 inner outer World world; \
                 replaceable Gain block1 constrainedby SISO(u = 0); \
               equation y = block1.y; block1.u = u; end Line; \
               model Top Line line(redeclare Gain block1(k = 2)); end Top; \
             end Lib;",
        )
        .unwrap();
        let angle = classes.iter().find(|c| c.name == "Lib.Angle").unwrap();
        assert_eq!(angle.alias_of.as_ref().unwrap().0, "Real");
        let line = classes.iter().find(|c| c.name == "Lib.Line").unwrap();
        // `inner outer` owns the instance: it is the `inner` half that
        // decides whether variables exist.
        let world = line.components.iter().find(|c| c.name == "world").unwrap();
        assert_eq!(world.scope, Scope::Inner);
        let top = classes.iter().find(|c| c.name == "Lib.Top").unwrap();
        assert_eq!(top.components[0].redeclares[0].name, "block1");
    }

    #[test]
    fn parses_the_discrete_layer() {
        let m = parse_model(
            "model M \
               discrete Real held; \
               Boolean on(start = false); \
               Integer count(start = 0); \
               Real x(start = 0, fixed = true); \
             equation \
               der(x) = 1; \
               when x > 1 then \
                 on = true; \
                 held = pre(held) + 1 \"one more\"; \
                 count = pre(count) + 1 annotation (Dialog()); \
               elsewhen x > 2 then \
                 on = false; \
                 held = pre(held); \
                 count = pre(count); \
               end when; \
             end M;",
        )
        .unwrap();
        let held = m.components.iter().find(|c| c.name == "held").unwrap();
        assert_eq!(held.variability, Variability::Discrete);
        let clause = &m.when_clauses[0];
        assert_eq!(clause.branches.len(), 2, "elsewhen is a second branch");
        assert_eq!(clause.branches[0].actions.len(), 3);
        assert!(matches!(
            clause.branches[1].actions[0],
            WhenAction::Assign(ref name, Expr::Bool(false)) if name == "on"
        ));

        // reinit and terminate still parse alongside the equations.
        let mixed = parse_model(
            "model M Real x(start = 1); Real state; equation der(x) = -1; \
             when x < 0 then reinit(x, 1); state = 1; terminate(\"done\"); end when; end M;",
        )
        .unwrap();
        assert_eq!(mixed.when_clauses[0].branches[0].actions.len(), 3);
    }

    #[test]
    fn parses_the_rarer_spellings() {
        // A renaming import, `.^`, a redeclare with constrainedby in a
        // modifier list, and error paths of lists that end wrongly.
        let m = parse_model(
            "package Lib model G parameter Real k = 1; Real u; Real y;              equation y = k * u; end G; end Lib;              model M import L = Lib; L.G g(k = 2); Real v[2]; Real w[2];              equation g.u = time; v = {2, 3}; w = v .^ 2; end M;",
        )
        .unwrap();
        assert!(m.components.iter().any(|c| c.name == "g.y"));
        assert_eq!(
            m.equations
                .iter()
                .filter(|e| format!("{:?}", e.rhs).contains("Pow"))
                .count(),
            2
        );

        let redeclared = parse_file(
            "package P partial model B Real y; end B;              model G extends B; parameter Real k = 1; equation y = k; end G;              model H replaceable G inner_block constrainedby B; end H;              model T H h(redeclare G inner_block(k = 2) constrainedby B(y = 0)); end T;              end P;",
        )
        .unwrap();
        let t = redeclared.iter().find(|c| c.name == "P.T").unwrap();
        assert_eq!(t.components[0].redeclares[0].name, "inner_block");

        // Ends that are neither comma nor the closing bracket.
        assert!(err_of("model M Real v[2 3]; end M;").contains("`,` or `]`"));
        assert!(
            err_of("model M Real v[2]; Real x; equation x = v[1 2]; end M;").contains("`,` or `]`")
        );
        assert!(err_of(
            "package P type K = Real(start = 1 min = 0); end P; model M Real x; end M;"
        )
        .contains("`,` or `)`"));
        // A subscripted member path: `points[1].x.y`.
        let member = parse_model(
            "record Q record R Real y; end R; end Q;              record P Real x; end P;              model M P points[2]; Real s;              equation points[1].x = 1; points[2].x = 2; s = points[1].x; end M;",
        )
        .unwrap();
        assert!(member.components.iter().any(|c| c.name == "points[2].x"));
    }

    #[test]
    fn parses_logic_precedence() {
        // a or b and not c  ==  a or (b and (not c))
        let m = parse_model(
            "model M Real a; Real b; Real c; Real y; equation \
             y = if a > 0 or b > 0 and not c > 0 then 1 else 0; end M;",
        )
        .unwrap();
        let Expr::If(cond, _, _) = &m.equations[0].rhs else {
            panic!("expected If: {:?}", m.equations[0].rhs)
        };
        let Expr::Or(_, or_rhs) = cond.as_ref() else {
            panic!("expected Or at the top level: {cond:?}")
        };
        assert!(matches!(or_rhs.as_ref(), Expr::And(_, _)));
    }
}
