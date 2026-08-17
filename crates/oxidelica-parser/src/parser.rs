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
    let tokens = lex(source).map_err(|e| ParseError {
        message: e.message,
        line: e.line,
    })?;
    let mut parser = Parser { tokens, pos: 0 };
    let mut classes = Vec::new();
    while parser.peek() != &Token::Eof {
        if parser.peek() == &Token::Within {
            parser.bump();
            if matches!(parser.peek(), Token::Ident(_)) {
                parser.dotted_name("within namespace")?;
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
        flatten_packages(class, &mut classes);
    }
    if classes.is_empty() {
        return Err(ParseError {
            message: "no class definitions in file".into(),
            line: 1,
        });
    }
    Ok(classes)
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
    let own = parse_file(source)?;
    // The model to simulate is the last one written at the top level of
    // the file; models nested inside other classes are components' types,
    // not entry points.
    let top = own
        .iter()
        .rev()
        .find(|c| c.kind == ClassKind::Model && !c.partial && !c.name.contains('.'))
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
}

/// What a modifier list contributes: value modifiers by (possibly
/// dotted) name, plus the redeclarations found among them.
type Modifications = (Vec<(String, Expr)>, Vec<Redeclare>);

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

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    /// The token `ahead` positions past the current one, clamped to the
    /// end-of-file marker. Used where a prefix keyword alone does not
    /// say whether a class or a component follows.
    fn peek_ahead(&self, ahead: usize) -> &Token {
        let index = (self.pos + ahead).min(self.tokens.len() - 1);
        &self.tokens[index].token
    }

    fn line(&self) -> u32 {
        self.tokens[self.pos].line
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].token.clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, expected: &Token, context: &str) -> Result<(), ParseError> {
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

    fn err(&self, message: String) -> ParseError {
        ParseError {
            message,
            line: self.line(),
        }
    }

    fn ident(&mut self, context: &str) -> Result<String, ParseError> {
        match self.peek().clone() {
            Token::Ident(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(self.err(format!("expected identifier ({context}), found `{other}`"))),
        }
    }

    fn opt_string(&mut self) -> Option<String> {
        if let Token::Str(s) = self.peek().clone() {
            self.bump();
            Some(s)
        } else {
            None
        }
    }

    fn class_def(&mut self) -> Result<ClassItem, ParseError> {
        // The prefixes matter for a short class definition, which may be
        // replaced from outside; on a full nested class they are noted
        // and otherwise carry no meaning here.
        let mut replaceable = false;
        let mut redeclaration = false;
        loop {
            match self.peek() {
                Token::Replaceable => replaceable = true,
                Token::Redeclare => redeclaration = true,
                Token::Final => {}
                _ => break,
            }
            self.bump();
        }
        let partial = if self.peek() == &Token::Partial {
            self.bump();
            true
        } else {
            false
        };
        let kind = match self.bump() {
            Token::Model => ClassKind::Model,
            Token::Connector => ClassKind::Connector,
            Token::Record => ClassKind::Record,
            Token::Function => ClassKind::Function,
            Token::Package => ClassKind::Package,
            Token::Type => ClassKind::Type,
            other => return Err(self.err(format!("expected a class definition, found `{other}`"))),
        };
        let name = self.ident("class name")?;

        // `package Medium = Media.Water constrainedby PartialMedium;` -
        // a short class definition: the enclosing class gets a local
        // name for another class, replaceable when marked so.
        if kind != ClassKind::Type && self.peek() == &Token::Assign {
            self.bump();
            let target = self.dotted_name("aliased class")?;
            if self.peek() == &Token::LParen {
                // Modifiers on the target are parsed and set aside: the
                // alias itself carries no component to modify.
                self.modifier_list()?;
            }
            let mut constrained_by = None;
            if self.peek() == &Token::ConstrainedBy {
                self.bump();
                constrained_by = Some(self.dotted_name("constraining class")?);
                if self.peek() == &Token::LParen {
                    self.modifier_list()?;
                }
            }
            self.opt_string();
            if self.peek() == &Token::Annotation {
                self.annotation_body(&mut Experiment::default())?;
            }
            self.expect(&Token::Semi, "semicolon after the class alias")?;
            return Ok(ClassItem::Alias(ClassAlias {
                name,
                target,
                replaceable,
                redeclaration,
                constrained_by,
            }));
        }

        // `type Voltage = Real(start = 0);` or
        // `type Init = enumeration(NoInit, SteadyState);`
        let mut alias_of = None;
        let mut enumeration = Vec::new();
        if kind == ClassKind::Type {
            self.expect(&Token::Assign, "`=` in a type alias")?;
            if self.peek() == &Token::Enumeration {
                enumeration = self.enumeration_literals()?;
            } else {
                let base = self.dotted_name("aliased type")?;
                let modifiers = if self.peek() == &Token::LParen {
                    self.type_attributes()?
                } else {
                    Vec::new()
                };
                alias_of = Some((base, modifiers));
            }
            self.opt_string();
            if self.peek() == &Token::Annotation {
                self.annotation_body(&mut Experiment::default())?;
            }
            self.expect(&Token::Semi, "semicolon after the type alias")?;
            return Ok(ClassItem::Class(Box::new(ClassDef {
                kind,
                name,
                partial,
                alias_of,
                enumeration,
                nested: Vec::new(),
                imports: Vec::new(),
                description: None,
                components: Vec::new(),
                extends: Vec::new(),
                equations: Vec::new(),
                initial_equations: Vec::new(),
                for_equations: Vec::new(),
                if_equations: Vec::new(),
                algorithm: Vec::new(),
                connects: Vec::new(),
                when_clauses: Vec::new(),
                experiment: Experiment::default(),
                class_aliases: Vec::new(),
            })));
        }

        let description = self.opt_string();

        let mut nested = Vec::new();
        let mut class_aliases = Vec::new();
        let mut imports = Vec::new();
        let mut components = Vec::new();
        let mut extends = Vec::new();
        let mut equations = Vec::new();
        let mut connects = Vec::new();
        let mut when_clauses = Vec::new();
        let mut for_equations = Vec::new();
        let mut if_equations = Vec::new();
        let mut algorithm = Vec::new();
        let mut initial_equations = Vec::new();
        let mut experiment = Experiment::default();
        let mut in_equations = false;
        // `initial equation` holds equations that describe the state the
        // model starts from rather than how it moves.
        let mut in_initial = false;

        loop {
            match self.peek() {
                Token::End => {
                    self.bump();
                    let end_name = self.ident("name after end")?;
                    if end_name != name {
                        return Err(
                            self.err(format!("end {end_name}; does not match class name {name}"))
                        );
                    }
                    self.expect(&Token::Semi, "semicolon after end")?;
                    break;
                }
                Token::Equation => {
                    self.bump();
                    in_equations = true;
                    in_initial = false;
                }
                // `initial` is not a keyword: `initial()` is a built-in
                // of the event layer, so the section is recognized by
                // the pair of tokens.
                Token::Ident(word)
                    if word == "initial" && self.peek_ahead(1) == &Token::Equation =>
                {
                    self.bump();
                    self.bump();
                    in_equations = true;
                    in_initial = true;
                }
                Token::Algorithm => {
                    self.bump();
                    algorithm.extend(self.statements()?);
                }
                Token::For => {
                    for_equations.push(self.for_equation()?);
                }
                Token::If if in_equations => {
                    if_equations.push(self.if_equation()?);
                }
                Token::Annotation => {
                    self.parse_annotation(&mut experiment)?;
                }
                Token::When => {
                    when_clauses.push(self.when_clause()?);
                }
                Token::Extends => {
                    extends.push(self.extends_clause()?);
                }
                Token::Connect => {
                    connects.push(self.connect_clause()?);
                }
                Token::Protected => {
                    self.bump();
                }
                Token::Import => {
                    imports.push(self.import_clause()?);
                }
                Token::Model
                | Token::Connector
                | Token::Record
                | Token::Function
                | Token::Package
                | Token::Type
                | Token::Partial => match self.class_def()? {
                    ClassItem::Class(class) => nested.push(*class),
                    ClassItem::Alias(alias) => class_aliases.push(alias),
                },
                // `replaceable`/`redeclare` introduce either a nested
                // class or a component; the next token decides.
                Token::Replaceable | Token::Redeclare
                    if matches!(
                        self.peek_ahead(1),
                        Token::Model
                            | Token::Connector
                            | Token::Record
                            | Token::Function
                            | Token::Package
                            | Token::Type
                            | Token::Partial
                    ) =>
                {
                    match self.class_def()? {
                        ClassItem::Class(class) => nested.push(*class),
                        ClassItem::Alias(alias) => class_aliases.push(alias),
                    }
                }
                Token::Eof => return Err(self.err("unexpected end of file: missing end".into())),
                // `assert(condition, "message")` is a runtime check;
                // it is accepted and skipped rather than rejected.
                Token::Ident(name) if in_equations && name == "assert" => {
                    self.bump();
                    self.expect(&Token::LParen, "parenthesis after assert")?;
                    let mut depth = 1usize;
                    while depth > 0 {
                        match self.bump() {
                            Token::LParen => depth += 1,
                            Token::RParen => depth -= 1,
                            Token::Eof => return Err(self.err("unterminated assert".into())),
                            _ => {}
                        }
                    }
                    self.expect(&Token::Semi, "semicolon after assert")?;
                }
                _ => {
                    if in_initial {
                        initial_equations.push(self.equation_item()?);
                    } else if in_equations {
                        equations.push(self.equation_item()?);
                    } else {
                        components.push(self.declaration()?);
                    }
                }
            }
        }

        Ok(ClassItem::Class(Box::new(ClassDef {
            kind,
            name,
            partial,
            alias_of,
            enumeration,
            nested,
            imports,
            description,
            components,
            extends,
            equations,
            initial_equations,
            for_equations,
            if_equations,
            algorithm,
            connects,
            when_clauses,
            experiment,
            class_aliases,
        })))
    }

    /// `import A.B.C;` or `import D = A.B.C;`
    fn import_clause(&mut self) -> Result<(String, String), ParseError> {
        self.expect(&Token::Import, "import")?;
        let first = self.ident("imported name")?;
        if self.peek() == &Token::Assign {
            self.bump();
            let target = self.dotted_name("import target")?;
            self.expect(&Token::Semi, "semicolon after import")?;
            return Ok((first, target));
        }
        let mut target = first;
        while self.peek() == &Token::Dot {
            self.bump();
            target.push('.');
            target.push_str(&self.ident("name after dot")?);
        }
        self.expect(&Token::Semi, "semicolon after import")?;
        let local = target
            .rsplit('.')
            .next()
            .expect("a dotted name has segments")
            .to_string();
        Ok((local, target))
    }

    /// A dotted class name: `Modelica.Electrical.Analog.Basic.Resistor`.
    fn dotted_name(&mut self, context: &str) -> Result<String, ParseError> {
        let mut name = self.ident(context)?;
        while self.peek() == &Token::Dot {
            self.bump();
            name.push('.');
            name.push_str(&self.ident(context)?);
        }
        Ok(name)
    }

    /// Attribute defaults of a `type` alias: `(start = 0, fixed = true)`.
    fn type_attributes(&mut self) -> Result<Vec<(String, Expr)>, ParseError> {
        self.expect(&Token::LParen, "type attributes")?;
        let mut out = Vec::new();
        loop {
            let name = self.ident("attribute name")?;
            self.expect(&Token::Assign, "`=` in a type attribute")?;
            // Unit strings and similar descriptive attributes are kept
            // as opaque text and ignored by the compiler.
            let value = match self.peek().clone() {
                Token::Str(_) => {
                    self.bump();
                    Expr::Number(0.0)
                }
                Token::True => {
                    self.bump();
                    Expr::Bool(true)
                }
                Token::False => {
                    self.bump();
                    Expr::Bool(false)
                }
                _ => self.expr()?,
            };
            if !matches!(name.as_str(), "unit" | "quantity" | "displayUnit") {
                out.push((name, value));
            }
            match self.bump() {
                Token::Comma => continue,
                Token::RParen => break,
                other => {
                    return Err(self.err(format!(
                        "expected `,` or `)` in type attributes, found `{other}`"
                    )))
                }
            }
        }
        Ok(out)
    }

    /// `for <var> in <lo>:<hi> loop <equations> end for;`
    fn for_equation(&mut self) -> Result<ForEquation, ParseError> {
        self.expect(&Token::For, "for")?;
        let variable = self.ident("loop variable")?;
        self.expect(&Token::In, "in after the loop variable")?;
        let lower = self.expr()?;
        self.expect(&Token::Colon, "`:` in the loop range")?;
        let upper = self.expr()?;
        self.expect(&Token::Loop, "loop after the range")?;
        let mut body = Vec::new();
        while self.peek() != &Token::End {
            match self.peek() {
                Token::Eof => return Err(self.err("unterminated for equation".into())),
                Token::For => body.push(ForBody::Nested(self.for_equation()?)),
                Token::Connect => {
                    let (a, b) = self.connect_clause()?;
                    body.push(ForBody::Connect(a, b));
                }
                _ => body.push(ForBody::Equation(self.equation_item()?)),
            }
        }
        self.expect(&Token::End, "end for")?;
        self.expect(&Token::For, "for after end")?;
        self.expect(&Token::Semi, "semicolon after end for")?;
        if body.is_empty() {
            return Err(self.err("for equation has no body".into()));
        }
        Ok(ForEquation {
            variable,
            range: (lower, upper),
            body,
        })
    }

    /// `if <cond> then <equations> [elseif …] [else …] end if;` in an
    /// equation section.
    fn if_equation(&mut self) -> Result<IfEquation, ParseError> {
        self.expect(&Token::If, "if")?;
        let mut branches = Vec::new();
        loop {
            let condition = self.expr()?;
            self.expect(&Token::Then, "then after the condition of an if equation")?;
            let (equations, connects) = self.branch_body()?;
            branches.push(IfBranch {
                condition: Some(condition),
                equations,
                connects,
            });
            match self.peek() {
                Token::ElseIf => {
                    self.bump();
                }
                Token::Else => {
                    self.bump();
                    let (equations, connects) = self.branch_body()?;
                    branches.push(IfBranch {
                        condition: None,
                        equations,
                        connects,
                    });
                    break;
                }
                _ => break,
            }
        }
        self.expect(&Token::End, "end if")?;
        self.expect(&Token::If, "if after end")?;
        self.expect(&Token::Semi, "semicolon after end if")?;
        if branches
            .iter()
            .all(|b| b.equations.is_empty() && b.connects.is_empty())
        {
            return Err(self.err("if equation has no equations".into()));
        }
        Ok(IfEquation { branches })
    }

    /// Equations and `connect` statements of one branch, up to the next
    /// `elseif`, `else` or `end`.
    fn branch_body(&mut self) -> Result<BranchBody, ParseError> {
        let mut equations = Vec::new();
        let mut connects = Vec::new();
        loop {
            match self.peek() {
                Token::ElseIf | Token::Else | Token::End => break,
                Token::Eof => return Err(self.err("unterminated if equation".into())),
                Token::Connect => connects.push(self.connect_clause()?),
                _ => equations.push(self.equation_item()?),
            }
        }
        Ok((equations, connects))
    }

    /// An algorithm section: assignments, `if` and `for` statements, up
    /// to whatever ends the section.
    fn statements(&mut self) -> Result<Vec<Statement>, ParseError> {
        let mut out = Vec::new();
        loop {
            match self.peek() {
                Token::Ident(_) => {
                    let target = self.component_ref()?;
                    // `c[i] := ...` assigns one element of an array.
                    let mut subscripts = Vec::new();
                    if self.peek() == &Token::LBracket {
                        self.bump();
                        loop {
                            subscripts.push(self.expr()?);
                            match self.bump() {
                                Token::Comma => continue,
                                Token::RBracket => break,
                                other => {
                                    return Err(self.err(format!(
                                        "expected `,` or `]` in a subscript, found `{other}`"
                                    )))
                                }
                            }
                        }
                    }
                    self.expect(&Token::Becomes, "`:=` in assignment")?;
                    let value = self.expr()?;
                    self.opt_string();
                    if self.peek() == &Token::Annotation {
                        self.annotation_body(&mut Experiment::default())?;
                    }
                    self.expect(&Token::Semi, "semicolon after assignment")?;
                    out.push(Statement::Assign(target, subscripts, value));
                }
                Token::If => out.push(self.if_statement()?),
                Token::For => out.push(self.for_statement()?),
                // A `while` has no trip count the compiler can see, so
                // it cannot be executed into equations.
                Token::While => {
                    return Err(self.err(
                        "`while` inside an algorithm is not supported: use a `for` with constant bounds"
                            .into(),
                    ))
                }
                _ => break,
            }
        }
        Ok(out)
    }

    /// `if c then … elseif … else … end if;` inside an algorithm.
    fn if_statement(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::If, "if")?;
        let mut branches = Vec::new();
        loop {
            let condition = self.expr()?;
            self.expect(&Token::Then, "then after the condition of an if statement")?;
            branches.push(StatementBranch {
                condition: Some(condition),
                body: self.statements()?,
            });
            match self.peek() {
                Token::ElseIf => {
                    self.bump();
                }
                Token::Else => {
                    self.bump();
                    branches.push(StatementBranch {
                        condition: None,
                        body: self.statements()?,
                    });
                    break;
                }
                _ => break,
            }
        }
        self.expect(&Token::End, "end if")?;
        self.expect(&Token::If, "if after end")?;
        self.expect(&Token::Semi, "semicolon after end if")?;
        Ok(Statement::If(branches))
    }

    /// `for i in lo:hi loop … end for;` inside an algorithm.
    fn for_statement(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::For, "for")?;
        let variable = self.ident("loop variable")?;
        self.expect(&Token::In, "in after the loop variable")?;
        let lower = self.expr()?;
        self.expect(&Token::Colon, "`:` in the loop range")?;
        let upper = self.expr()?;
        self.expect(&Token::Loop, "loop after the range")?;
        let body = self.statements()?;
        self.expect(&Token::End, "end for")?;
        self.expect(&Token::For, "for after end")?;
        self.expect(&Token::Semi, "semicolon after end for")?;
        if body.is_empty() {
            return Err(self.err("for statement has no body".into()));
        }
        Ok(Statement::For(variable, (lower, upper), body))
    }

    /// `enumeration(NoInit, SteadyState "start at steady state")`.
    fn enumeration_literals(&mut self) -> Result<Vec<String>, ParseError> {
        self.expect(&Token::Enumeration, "enumeration")?;
        self.expect(&Token::LParen, "parenthesis after enumeration")?;
        let mut literals = Vec::new();
        loop {
            literals.push(self.ident("enumeration literal")?);
            self.opt_string();
            match self.bump() {
                Token::Comma => continue,
                Token::RParen => break,
                other => {
                    return Err(self.err(format!(
                        "expected `,` or `)` in an enumeration, found `{other}`"
                    )))
                }
            }
        }
        Ok(literals)
    }

    /// `extends Base(mod = expr, redeclare Type name, ...);`
    fn extends_clause(&mut self) -> Result<Extend, ParseError> {
        self.expect(&Token::Extends, "extends")?;
        let base = self.dotted_name("base class name")?;
        let (modifiers, redeclares) = if self.peek() == &Token::LParen {
            self.modifier_list()?
        } else {
            (Vec::new(), Vec::new())
        };
        if self.peek() == &Token::Annotation {
            self.annotation_body(&mut Experiment::default())?;
        }
        self.expect(&Token::Semi, "semicolon after extends")?;
        Ok(Extend {
            base,
            modifiers,
            redeclares,
        })
    }

    /// `connect(a.b, c.d) annotation(...);` — the references may carry
    /// subscripts (`pins[i]`, `a[2].p`) or name whole arrays.
    fn connect_clause(&mut self) -> Result<(Expr, Expr), ParseError> {
        self.expect(&Token::Connect, "connect")?;
        self.expect(&Token::LParen, "parenthesis after connect")?;
        let left = self.connect_ref()?;
        self.expect(&Token::Comma, "comma in connect")?;
        let right = self.connect_ref()?;
        self.expect(&Token::RParen, "closing parenthesis of connect")?;
        if self.peek() == &Token::Annotation {
            self.annotation_body(&mut Experiment::default())?;
        }
        self.expect(&Token::Semi, "semicolon after connect")?;
        Ok((left, right))
    }

    /// One side of a `connect`: a dotted name, optionally subscripted,
    /// optionally followed by more of the path.
    fn connect_ref(&mut self) -> Result<Expr, ParseError> {
        let name = self.component_ref()?;
        if self.peek() != &Token::LBracket {
            return Ok(Expr::Ref(name));
        }
        self.bump();
        let mut subscripts = Vec::new();
        loop {
            subscripts.push(self.expr()?);
            match self.bump() {
                Token::Comma => continue,
                Token::RBracket => break,
                other => {
                    return Err(self.err(format!(
                        "expected `,` or `]` in a subscript, found `{other}`"
                    )))
                }
            }
        }
        let indexed = Expr::Index(Box::new(Expr::Ref(name)), subscripts);
        if self.peek() == &Token::Dot {
            self.bump();
            let path = self.component_ref()?;
            return Ok(Expr::Member(Box::new(indexed), path));
        }
        Ok(indexed)
    }

    /// A dotted component reference: `a.b.c`.
    fn component_ref(&mut self) -> Result<String, ParseError> {
        let mut name = self.ident("component reference")?;
        while self.peek() == &Token::Dot {
            self.bump();
            name.push('.');
            name.push_str(&self.ident("name after dot")?);
        }
        Ok(name)
    }

    /// `( name = expr, sub(name = expr), redeclare Type name, ... )` —
    /// component and `extends` modifiers.
    ///
    /// Nested modifiers are flattened into dotted names, so
    /// `inertia(J = 2, phi(start = 1))` yields `inertia.J` and
    /// `inertia.phi.start`; instantiation routes a dotted name to the
    /// child component or, for a primitive, to its attribute. Values
    /// that are strings (units, descriptive text) carry no meaning for
    /// the compiler and are dropped.
    fn modifier_list(&mut self) -> Result<Modifications, ParseError> {
        self.expect(&Token::LParen, "modifier list")?;
        let mut modifiers = Vec::new();
        let mut redeclares = Vec::new();
        // An empty list, `Interface()`, modifies nothing.
        if self.peek() == &Token::RParen {
            self.bump();
            return Ok((modifiers, redeclares));
        }
        loop {
            while matches!(self.peek(), Token::Final | Token::Each) {
                self.bump();
            }
            if self.peek() == &Token::Redeclare {
                redeclares.push(self.redeclaration()?);
            } else {
                let name = self.component_ref()?;
                if self.peek() == &Token::LParen {
                    let (nested, nested_redeclares) = self.modifier_list()?;
                    modifiers.extend(
                        nested
                            .into_iter()
                            .map(|(sub, value)| (format!("{name}.{sub}"), value)),
                    );
                    redeclares.extend(nested_redeclares.into_iter().map(|mut r| {
                        r.name = format!("{name}.{}", r.name);
                        r
                    }));
                }
                // A binding may follow a nested list: `x(unit = "m") = 3`.
                if self.peek() == &Token::Assign {
                    self.bump();
                    if let Some(value) = self.modifier_value()? {
                        modifiers.push((name, value));
                    }
                } else if !self.at_modifier_end() {
                    return Err(self.err(format!(
                        "expected `=` or a nested modifier list after `{name}`, found `{}`",
                        self.peek()
                    )));
                }
            }
            match self.bump() {
                Token::Comma => continue,
                Token::RParen => break,
                other => {
                    return Err(
                        self.err(format!("expected `,` or `)` in modifiers, found `{other}`"))
                    )
                }
            }
        }
        Ok((modifiers, redeclares))
    }

    /// Whether the current token closes a modifier or the whole list.
    fn at_modifier_end(&self) -> bool {
        matches!(self.peek(), Token::Comma | Token::RParen)
    }

    /// The value of one modifier. `None` means the value was a string:
    /// the compiler has no use for it, so the modifier is dropped.
    fn modifier_value(&mut self) -> Result<Option<Expr>, ParseError> {
        if matches!(self.peek(), Token::Str(_)) {
            self.bump();
            return Ok(None);
        }
        Ok(Some(self.expr()?))
    }

    /// `redeclare [replaceable] Type name(modifiers) [constrainedby C]`
    /// inside a modifier list.
    fn redeclaration(&mut self) -> Result<Redeclare, ParseError> {
        self.expect(&Token::Redeclare, "redeclare")?;
        while matches!(self.peek(), Token::Replaceable | Token::Final | Token::Each) {
            self.bump();
        }
        // `redeclare package Medium = Oil` swaps a class alias.
        if matches!(
            self.peek(),
            Token::Package | Token::Model | Token::Function | Token::Record | Token::Connector
        ) {
            self.bump();
            let name = self.ident("redeclared class name")?;
            self.expect(&Token::Assign, "`=` in a class redeclaration")?;
            let target = self.dotted_name("replacement class")?;
            if self.peek() == &Token::LParen {
                self.modifier_list()?;
            }
            if self.peek() == &Token::ConstrainedBy {
                self.bump();
                self.dotted_name("constraining class")?;
                if self.peek() == &Token::LParen {
                    self.modifier_list()?;
                }
            }
            self.opt_string();
            return Ok(Redeclare {
                name,
                type_name: target,
                modifiers: Vec::new(),
                class_level: true,
            });
        }
        let type_name = self.dotted_name("redeclared type")?;
        let name = self.ident("redeclared component name")?;
        let modifiers = if self.peek() == &Token::LParen {
            self.modifier_list()?.0
        } else {
            Vec::new()
        };
        if self.peek() == &Token::ConstrainedBy {
            self.bump();
            self.dotted_name("constraining type")?;
            if self.peek() == &Token::LParen {
                self.modifier_list()?;
            }
        }
        self.opt_string();
        Ok(Redeclare {
            name,
            type_name,
            modifiers,
            class_level: false,
        })
    }

    /// `when <cond> then <action>; … [elsewhen <cond> then …] end when;`
    ///
    /// A branch holds equations for discrete variables (`x = expr`),
    /// `reinit(state, expr)` and `terminate("message")`.
    fn when_clause(&mut self) -> Result<WhenClause, ParseError> {
        self.expect(&Token::When, "when")?;
        let mut branches = Vec::new();
        loop {
            let condition = self.expr()?;
            self.expect(&Token::Then, "then after when condition")?;
            let actions = self.when_actions()?;
            if actions.is_empty() {
                return Err(self.err("when branch has no actions".into()));
            }
            branches.push(WhenBranch { condition, actions });
            if self.peek() == &Token::ElseWhen {
                self.bump();
                continue;
            }
            break;
        }
        self.expect(&Token::End, "end when")?;
        self.expect(&Token::When, "when after end")?;
        self.expect(&Token::Semi, "semicolon after end when")?;
        Ok(WhenClause { branches })
    }

    /// The body of one `when` branch, up to `elsewhen` or `end`.
    fn when_actions(&mut self) -> Result<Vec<WhenAction>, ParseError> {
        let mut actions = Vec::new();
        loop {
            match self.peek() {
                Token::End | Token::ElseWhen => break,
                Token::Eof => return Err(self.err("unterminated when clause".into())),
                _ => {}
            }
            let target = self.component_ref()?;
            match (target.as_str(), self.peek()) {
                // `reinit(state, expr)` and `terminate("message")` act on
                // the solver rather than on a variable.
                ("reinit", Token::LParen) => {
                    self.bump();
                    let state = self.component_ref()?;
                    self.expect(&Token::Comma, "comma in reinit")?;
                    let value = self.expr()?;
                    self.expect(&Token::RParen, "closing parenthesis of reinit")?;
                    actions.push(WhenAction::Reinit(state, value));
                }
                ("terminate", Token::LParen) => {
                    self.bump();
                    let message = match self.bump() {
                        Token::Str(message) => message,
                        other => {
                            return Err(self.err(format!(
                                "terminate expects a string message, found `{other}`"
                            )))
                        }
                    };
                    self.expect(&Token::RParen, "closing parenthesis of terminate")?;
                    actions.push(WhenAction::Terminate(message));
                }
                // Anything else is an equation for a discrete variable.
                (_, _) => {
                    self.expect(&Token::Assign, "`=` in an equation inside when")?;
                    let value = self.expr()?;
                    actions.push(WhenAction::Assign(target, value));
                }
            }
            self.opt_string();
            if self.peek() == &Token::Annotation {
                self.annotation_body(&mut Experiment::default())?;
            }
            self.expect(&Token::Semi, "semicolon after the action")?;
        }
        Ok(actions)
    }

    fn declaration(&mut self) -> Result<Component, ParseError> {
        // Declaration prefixes may come in any order the specification
        // allows: `inner replaceable parameter Real k`.
        let mut variability = Variability::Continuous;
        let mut flow = false;
        let mut causality = Causality::None;
        let mut scope = Scope::Local;
        let mut replaceable = false;
        let mut redeclaration = false;
        loop {
            match self.peek() {
                Token::Parameter => variability = Variability::Parameter,
                Token::Constant => variability = Variability::Constant,
                Token::Discrete => variability = Variability::Discrete,
                Token::Flow => flow = true,
                Token::Input => causality = Causality::Input,
                Token::Output => causality = Causality::Output,
                Token::Inner => scope = Scope::Inner,
                // `inner outer x` owns the instance and refers to the
                // enclosing one; owning is what creates the variables.
                Token::Outer if scope != Scope::Inner => scope = Scope::Outer,
                Token::Outer => {}
                Token::Replaceable => replaceable = true,
                Token::Redeclare => redeclaration = true,
                Token::Final | Token::Each => {}
                _ => break,
            }
            self.bump();
        }

        let type_name = self.dotted_name("component type")?;
        let name = self.ident("component name")?;
        // `Real T[N, 3]` — dimensions are constant expressions.
        let mut dimensions = Vec::new();
        if self.peek() == &Token::LBracket {
            self.bump();
            loop {
                dimensions.push(self.expr()?);
                match self.bump() {
                    Token::Comma => continue,
                    Token::RBracket => break,
                    other => {
                        return Err(self.err(format!(
                            "expected `,` or `]` in array dimensions, found `{other}`"
                        )))
                    }
                }
            }
        }

        let mut start = None;
        let mut fixed = None;
        let mut modifiers = Vec::new();
        let mut redeclares = Vec::new();
        if self.peek() == &Token::LParen {
            if matches!(type_name.as_str(), "Real" | "Integer" | "Boolean") {
                self.bump();
                loop {
                    while matches!(self.peek(), Token::Final | Token::Each) {
                        self.bump();
                    }
                    let attr = self.ident("attribute name")?;
                    self.expect(&Token::Assign, "`=` in attribute")?;
                    match attr.as_str() {
                        "start" => start = Some(self.expr()?),
                        "fixed" => {
                            fixed = Some(match self.bump() {
                                Token::True => true,
                                Token::False => false,
                                other => {
                                    return Err(self
                                        .err(format!("fixed expects true/false, found `{other}`")))
                                }
                            });
                        }
                        // The remaining attributes (unit, min, max,
                        // nominal, stateSelect, …) describe the variable
                        // rather than the equations: parsed and dropped.
                        _ => {
                            self.modifier_value()?;
                        }
                    }
                    match self.peek() {
                        Token::Comma => {
                            self.bump();
                        }
                        Token::RParen => {
                            self.bump();
                            break;
                        }
                        other => {
                            return Err(self.err(format!(
                                "expected `,` or `)` in attributes, found `{other}`"
                            )))
                        }
                    }
                }
            } else {
                (modifiers, redeclares) = self.modifier_list()?;
            }
        }

        let binding = if self.peek() == &Token::Assign {
            self.bump();
            Some(self.expr()?)
        } else {
            None
        };

        // `constrainedby Interface(...)` and the condition `if expr` may
        // follow the declaration, in either order.
        let mut constrained_by = None;
        let mut condition = None;
        loop {
            match self.peek() {
                Token::ConstrainedBy => {
                    self.bump();
                    constrained_by = Some(self.dotted_name("constraining type")?);
                    if self.peek() == &Token::LParen {
                        self.modifier_list()?;
                    }
                }
                Token::If => {
                    self.bump();
                    condition = Some(self.expr()?);
                }
                _ => break,
            }
        }

        let description = self.opt_string();
        if self.peek() == &Token::Annotation {
            self.annotation_body(&mut Experiment::default())?;
        }
        self.expect(&Token::Semi, "semicolon after declaration")?;

        Ok(Component {
            name,
            type_name,
            flow,
            dimensions,
            causality,
            modifiers,
            variability,
            start,
            fixed,
            binding,
            description,
            scope,
            replaceable,
            constrained_by,
            condition,
            redeclares,
            redeclaration,
        })
    }

    fn equation_item(&mut self) -> Result<EquationItem, ParseError> {
        let lhs = self.expr()?;
        self.expect(&Token::Assign, "`=` in equation")?;
        let rhs = self.expr()?;
        self.opt_string();
        if self.peek() == &Token::Annotation {
            self.annotation_body(&mut Experiment::default())?;
        }
        self.expect(&Token::Semi, "semicolon after equation")?;
        Ok(EquationItem { lhs, rhs })
    }

    /// A class-level `annotation ( ... ) ;`.
    fn parse_annotation(&mut self, experiment: &mut Experiment) -> Result<(), ParseError> {
        self.annotation_body(experiment)?;
        self.expect(&Token::Semi, "semicolon after annotation")?;
        Ok(())
    }

    /// `annotation ( ... )` without its terminator — declarations,
    /// equations and `connect` statements carry one before the
    /// semicolon. Parsed tolerantly: only
    /// `experiment(StopTime=…, Interval=…, Tolerance=…)` is extracted,
    /// everything else is skipped by balancing parentheses.
    fn annotation_body(&mut self, experiment: &mut Experiment) -> Result<(), ParseError> {
        self.expect(&Token::Annotation, "annotation")?;
        self.expect(&Token::LParen, "parenthesis after annotation")?;
        let mut depth = 1usize;
        while depth > 0 {
            match self.peek().clone() {
                Token::Eof => return Err(self.err("unterminated annotation".into())),
                Token::LParen | Token::LBrace | Token::LBracket => {
                    depth += 1;
                    self.bump();
                }
                Token::RParen | Token::RBrace | Token::RBracket => {
                    depth -= 1;
                    self.bump();
                }
                Token::Ident(name) if depth == 1 && name == "experiment" => {
                    self.bump();
                    self.expect(&Token::LParen, "parenthesis after experiment")?;
                    loop {
                        let key = self.ident("experiment parameter")?;
                        self.expect(&Token::Assign, "`=` in experiment")?;
                        let value = self.number_literal()?;
                        match key.as_str() {
                            "StopTime" => experiment.stop_time = Some(value),
                            "Interval" => experiment.interval = Some(value),
                            "Tolerance" => experiment.tolerance = Some(value),
                            "StartTime" if value != 0.0 => {
                                return Err(self.err("M0: StartTime must be 0".into()));
                            }
                            "StartTime" => {}
                            _ => {} // Unknown keys are silently skipped.
                        }
                        match self.bump() {
                            Token::Comma => continue,
                            Token::RParen => break,
                            other => {
                                return Err(self.err(format!(
                                    "expected `,` or `)` in experiment, found `{other}`"
                                )))
                            }
                        }
                    }
                }
                _ => {
                    self.bump();
                }
            }
        }
        Ok(())
    }

    fn number_literal(&mut self) -> Result<f64, ParseError> {
        let negative = if self.peek() == &Token::Minus {
            self.bump();
            true
        } else {
            false
        };
        match self.bump() {
            Token::Number(n) => Ok(if negative { -n } else { n }),
            other => Err(self.err(format!("expected a number, found `{other}`"))),
        }
    }

    // --- expressions ---
    // Hierarchy per the Modelica specification:
    // expr -> if | logical_or; or -> and -> not -> relation -> arithmetic.

    fn expr(&mut self) -> Result<Expr, ParseError> {
        if self.peek() == &Token::If {
            return self.if_expression();
        }
        self.logical_or()
    }

    /// `if c then a {elseif c2 then b} else d` — becomes nested `If`s.
    fn if_expression(&mut self) -> Result<Expr, ParseError> {
        self.expect(&Token::If, "if")?;
        self.if_body()
    }

    /// Shared body for if and elseif:
    /// `<condition> then <expression> (elseif … | else …)`.
    fn if_body(&mut self) -> Result<Expr, ParseError> {
        let condition = self.expr()?;
        self.expect(&Token::Then, "then after condition")?;
        let then_branch = self.expr()?;
        let else_branch = match self.bump() {
            Token::ElseIf => self.if_body()?,
            Token::Else => self.expr()?,
            other => {
                return Err(self.err(format!("expected else/elseif, found `{other}`")));
            }
        };
        Ok(Expr::If(
            Box::new(condition),
            Box::new(then_branch),
            Box::new(else_branch),
        ))
    }

    fn logical_or(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.logical_and()?;
        while self.peek() == &Token::Or {
            self.bump();
            let rhs = self.logical_and()?;
            lhs = Expr::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn logical_and(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.logical_not()?;
        while self.peek() == &Token::And {
            self.bump();
            let rhs = self.logical_not()?;
            lhs = Expr::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn logical_not(&mut self) -> Result<Expr, ParseError> {
        if self.peek() == &Token::Not {
            self.bump();
            let inner = self.logical_not()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.relation()
    }

    fn relation(&mut self) -> Result<Expr, ParseError> {
        let lhs = self.arith()?;
        let op = match self.peek() {
            Token::Lt => RelOp::Lt,
            Token::Le => RelOp::Le,
            Token::Gt => RelOp::Gt,
            Token::Ge => RelOp::Ge,
            Token::EqEq => RelOp::Eq,
            Token::Ne => RelOp::Ne,
            _ => return Ok(lhs),
        };
        self.bump();
        let rhs = self.arith()?;
        Ok(Expr::Rel(op, Box::new(lhs), Box::new(rhs)))
    }

    fn arith(&mut self) -> Result<Expr, ParseError> {
        // Unary sign at the addition level.
        let leading_neg = match self.peek() {
            Token::Minus => {
                self.bump();
                true
            }
            Token::Plus => {
                self.bump();
                false
            }
            _ => false,
        };
        let mut lhs = self.term()?;
        if leading_neg {
            lhs = Expr::Neg(Box::new(lhs));
        }
        loop {
            let (op, elementwise) = match self.peek() {
                Token::Plus => (BinOp::Add, false),
                Token::Minus => (BinOp::Sub, false),
                Token::DotPlus => (BinOp::Add, true),
                Token::DotMinus => (BinOp::Sub, true),
                _ => break,
            };
            self.bump();
            let rhs = self.term()?;
            lhs = if elementwise {
                Expr::Elementwise(op, Box::new(lhs), Box::new(rhs))
            } else {
                Expr::Bin(op, Box::new(lhs), Box::new(rhs))
            };
        }
        Ok(lhs)
    }

    fn term(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.factor()?;
        loop {
            let (op, elementwise) = match self.peek() {
                Token::Star => (BinOp::Mul, false),
                Token::Slash => (BinOp::Div, false),
                Token::DotStar => (BinOp::Mul, true),
                Token::DotSlash => (BinOp::Div, true),
                _ => break,
            };
            self.bump();
            let rhs = self.factor()?;
            lhs = if elementwise {
                Expr::Elementwise(op, Box::new(lhs), Box::new(rhs))
            } else {
                Expr::Bin(op, Box::new(lhs), Box::new(rhs))
            };
        }
        Ok(lhs)
    }

    fn factor(&mut self) -> Result<Expr, ParseError> {
        let base = self.primary()?;
        match self.peek() {
            Token::Caret => {
                self.bump();
                let exponent = self.primary()?;
                Ok(Expr::Bin(BinOp::Pow, Box::new(base), Box::new(exponent)))
            }
            Token::DotCaret => {
                self.bump();
                let exponent = self.primary()?;
                Ok(Expr::Elementwise(
                    BinOp::Pow,
                    Box::new(base),
                    Box::new(exponent),
                ))
            }
            _ => Ok(base),
        }
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            Token::Number(n) => {
                self.bump();
                Ok(Expr::Number(n))
            }
            Token::True => {
                self.bump();
                Ok(Expr::Bool(true))
            }
            Token::False => {
                self.bump();
                Ok(Expr::Bool(false))
            }
            Token::LParen => {
                self.bump();
                let inner = self.expr()?;
                self.expect(&Token::RParen, "closing parenthesis")?;
                Ok(inner)
            }
            Token::LBrace => {
                self.bump();
                let mut items = Vec::new();
                if self.peek() != &Token::RBrace {
                    loop {
                        items.push(self.expr()?);
                        match self.peek() {
                            Token::Comma => {
                                self.bump();
                            }
                            _ => break,
                        }
                    }
                }
                self.expect(&Token::RBrace, "closing brace of an array")?;
                Ok(Expr::Array(items))
            }
            Token::Ident(name) => {
                self.bump();
                if name == "time" {
                    return Ok(Expr::Time);
                }
                let mut name = name;
                while self.peek() == &Token::Dot {
                    self.bump();
                    name.push('.');
                    name.push_str(&self.ident("name after dot")?);
                }
                if self.peek() == &Token::LBracket {
                    self.bump();
                    let mut subscripts = Vec::new();
                    loop {
                        subscripts.push(self.expr()?);
                        match self.bump() {
                            Token::Comma => continue,
                            Token::RBracket => break,
                            other => {
                                return Err(self.err(format!(
                                    "expected `,` or `]` in a subscript, found `{other}`"
                                )))
                            }
                        }
                    }
                    let indexed = Expr::Index(Box::new(Expr::Ref(name)), subscripts);
                    if self.peek() == &Token::Dot {
                        self.bump();
                        let mut path = self.ident("member after `.`")?;
                        while self.peek() == &Token::Dot {
                            self.bump();
                            path.push('.');
                            path.push_str(&self.ident("member after `.`")?);
                        }
                        return Ok(Expr::Member(Box::new(indexed), path));
                    }
                    return Ok(indexed);
                }
                // A call may be qualified: `Lib.reverse(v)` names a
                // function the way everything else is named.
                if self.peek() == &Token::LParen {
                    self.bump();
                    let mut args = Vec::new();
                    if self.peek() != &Token::RParen {
                        loop {
                            args.push(self.expr()?);
                            match self.peek() {
                                Token::Comma => {
                                    self.bump();
                                }
                                _ => break,
                            }
                        }
                    }
                    self.expect(&Token::RParen, "closing parenthesis of call")?;
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Ref(name))
                }
            }
            other => Err(self.err(format!("unexpected token in expression: `{other}`"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // An attribute the compiler has no use for is dropped, not
        // rejected: the standard library sets these on most variables.
        let tolerated =
            parse_model("model M Real x(min = 0, max = 1, unit = \"m\", start = 2); end M;")
                .unwrap();
        assert_eq!(tolerated.components[0].start, Some(Expr::Number(2.0)));
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
