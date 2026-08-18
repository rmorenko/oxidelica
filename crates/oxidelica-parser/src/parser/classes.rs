//! Class definitions: what a `model`, `package`, `record` or
//! `operator record` is made of, and the clauses that sit at the top of
//! one - `import`, `extends`, `annotation`.

use super::*;

impl Parser {
    pub(super) fn class_def(&mut self) -> Result<ClassItem, ParseError> {
        // The prefixes matter for a short class definition, which may be
        // replaced from outside; on a full nested class they are noted
        // and otherwise carry no meaning here.
        let mut replaceable = false;
        let mut redeclaration = false;
        let mut operator_class = false;
        let mut encapsulated = false;
        loop {
            match self.peek() {
                Token::Replaceable => replaceable = true,
                Token::Redeclare => redeclaration = true,
                // `encapsulated` says the class may not see out of
                // itself: a simple name inside it is looked up here and
                // in its imports, never in an enclosing package.
                Token::Encapsulated => encapsulated = true,
                Token::Final | Token::Pure | Token::Impure => {}
                Token::Operator => operator_class = true,
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
        let expandable = if self.peek() == &Token::Expandable {
            self.bump();
            true
        } else {
            false
        };
        // `operator '+' ... end '+';` gathers the overloads of a
        // symbol: a package by another name, with the symbol for a
        // name and nothing between the keyword and it.
        let kind = if operator_class && matches!(self.peek(), Token::Ident(_)) {
            ClassKind::Package
        } else {
            match self.bump() {
                // `class` puts no restriction on what is inside it.
                Token::Model | Token::Block | Token::Class => ClassKind::Model,
                Token::Connector => ClassKind::Connector,
                Token::Record => ClassKind::Record,
                Token::Function => ClassKind::Function,
                Token::Package => ClassKind::Package,
                Token::Type => ClassKind::Type,
                other => {
                    return Err(self.err(format!("expected a class definition, found `{other}`")))
                }
            }
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
        let mut alias_unit = None;
        let mut enumeration = Vec::new();
        if kind == ClassKind::Type {
            self.expect(&Token::Assign, "`=` in a type alias")?;
            if self.peek() == &Token::Enumeration {
                enumeration = self.enumeration_literals()?;
            } else {
                let base = self.dotted_name("aliased type")?;
                let modifiers = if self.peek() == &Token::LParen {
                    let (modifiers, unit) = self.type_attributes()?;
                    alias_unit = unit;
                    modifiers
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
                encapsulated,
                expandable,
                alias_of,
                alias_unit,
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
                asserts: Vec::new(),
                transitions: Vec::new(),
                initial_state: None,
                connection_graph: Vec::new(),
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
        let mut asserts = Vec::new();
        let mut transitions = Vec::new();
        let mut initial_state = None;
        let mut connection_graph = Vec::new();
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
                // `initial` is both an operator and the head of a
                // section; the token after it says which.
                Token::Initial if self.peek_ahead(1) == &Token::Equation => {
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
                Token::Protected | Token::Public => {
                    self.bump();
                }
                // An implementation outside Modelica: better to say so
                // than to fail somewhere further in.
                Token::External => {
                    return Err(self.err(
                        "`external` is not supported: a function must have a Modelica body"
                            .to_string(),
                    ))
                }
                Token::Import => {
                    imports.extend(self.import_clause()?);
                }
                Token::Model
                | Token::Block
                | Token::Class
                | Token::Connector
                | Token::Record
                | Token::Function
                | Token::Package
                | Token::Type
                | Token::Partial
                | Token::Expandable
                | Token::Operator
                | Token::Encapsulated
                | Token::Pure
                | Token::Impure => match self.class_def()? {
                    ClassItem::Class(class) => nested.push(*class),
                    ClassItem::Alias(alias) => class_aliases.push(alias),
                },
                // `replaceable`/`redeclare` introduce either a nested
                // class or a component; the next token decides.
                Token::Replaceable | Token::Redeclare
                    if matches!(
                        self.peek_ahead(1),
                        Token::Model
                            | Token::Block
                            | Token::Class
                            | Token::Connector
                            | Token::Record
                            | Token::Function
                            | Token::Package
                            | Token::Type
                            | Token::Partial
                            | Token::Expandable
                            | Token::Operator
                            | Token::Encapsulated
                            | Token::Pure
                            | Token::Impure
                    ) =>
                {
                    match self.class_def()? {
                        ClassItem::Class(class) => nested.push(*class),
                        ClassItem::Alias(alias) => class_aliases.push(alias),
                    }
                }
                Token::Eof => return Err(self.err("unexpected end of file: missing end".into())),
                // `assert(condition, "message")` is a runtime check.
                // An optional third argument names the assertion level;
                // it is accepted and not distinguished.
                Token::Ident(name) if in_equations && name == "assert" => {
                    self.bump();
                    let (condition, message) = self.assert_arguments()?;
                    if self.peek() == &Token::Annotation {
                        self.annotation_body(&mut Experiment::default())?;
                    }
                    self.expect(&Token::Semi, "semicolon after assert")?;
                    asserts.push((condition, message));
                }
                // `initialState(s);` and
                // `transition(from, to, condition, ...);` draw a state
                // machine. They are written as equations and are not
                // equations at all: nothing is equated.
                Token::Ident(name) if in_equations && name == "initialState" => {
                    self.bump();
                    self.expect(&Token::LParen, "parenthesis after initialState")?;
                    let named = self.dotted_name("the state to start in")?;
                    if let Some(already) = &initial_state {
                        return Err(self.err(format!("this class already starts in `{already}`, so `{named}` is one initial state too many")));
                    }
                    initial_state = Some(named);
                    self.expect(&Token::RParen, "closing parenthesis of initialState")?;
                    if self.peek() == &Token::Annotation {
                        self.annotation_body(&mut Experiment::default())?;
                    }
                    self.expect(&Token::Semi, "semicolon after initialState")?;
                }
                Token::Ident(name) if in_equations && name == "transition" => {
                    self.bump();
                    transitions.push(self.transition_clause()?);
                }
                // `Connections.root(a);` and its relatives say how an
                // overconstrained graph is to be broken open.
                Token::Ident(name) if in_equations && name == "Connections" => {
                    connection_graph.push(self.connections_clause()?);
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
            encapsulated,
            expandable,
            alias_of,
            alias_unit,
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
            asserts,
            transitions,
            initial_state,
            connection_graph,
        })))
    }

    /// `import A.B.C;` or `import D = A.B.C;`
    pub(super) fn import_clause(&mut self) -> Result<Vec<(String, String)>, ParseError> {
        self.expect(&Token::Import, "import")?;
        let first = self.ident("imported name")?;
        if self.peek() == &Token::Assign {
            self.bump();
            let target = self.dotted_name("import target")?;
            self.expect(&Token::Semi, "semicolon after import")?;
            return Ok(vec![(first, target)]);
        }
        let mut target = first;
        loop {
            if self.peek() != &Token::Dot {
                break;
            }
            // `import A.B.{C, D};` names several members at once.
            if self.peek_at(1) == &Token::LBrace {
                self.bump();
                self.bump();
                let mut named = Vec::new();
                loop {
                    let member = self.ident("imported member")?;
                    named.push((member.clone(), format!("{target}.{member}")));
                    match self.bump() {
                        Token::Comma => continue,
                        Token::RBrace => break,
                        other => {
                            return Err(self.err(format!(
                                "expected `,` or `}}` in an import list, found `{other}`"
                            )))
                        }
                    }
                }
                self.expect(&Token::Semi, "semicolon after import")?;
                return Ok(named);
            }
            self.bump();
            target.push('.');
            target.push_str(&self.ident("name after dot")?);
        }
        // `import A.B.*;` makes the members of `A.B` known by their
        // own names. The lexer reads `.*` as one token, since that is
        // also how the elementwise operators are spelled.
        if self.peek() == &Token::DotStar {
            self.bump();
            self.expect(&Token::Semi, "semicolon after import")?;
            return Ok(vec![(WILDCARD_IMPORT.to_string(), target)]);
        }
        self.expect(&Token::Semi, "semicolon after import")?;
        let local = target
            .rsplit('.')
            .next()
            .expect("a dotted name has segments")
            .to_string();
        Ok(vec![(local, target)])
    }

    /// `extends Base(mod = expr, redeclare Type name, ...);`
    pub(super) fn extends_clause(&mut self) -> Result<Extend, ParseError> {
        self.expect(&Token::Extends, "extends")?;
        let base = self.dotted_name("base class name")?;
        let (modifiers, redeclares, _each, broken) = if self.peek() == &Token::LParen {
            self.modifier_list()?
        } else {
            (Vec::new(), Vec::new(), Vec::new(), Vec::new())
        };
        if self.peek() == &Token::Annotation {
            self.annotation_body(&mut Experiment::default())?;
        }
        self.expect(&Token::Semi, "semicolon after extends")?;
        Ok(Extend {
            base,
            modifiers,
            broken,
            redeclares,
        })
    }

    /// `enumeration(NoInit, SteadyState "start at steady state")`.
    pub(super) fn enumeration_literals(&mut self) -> Result<Vec<String>, ParseError> {
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

    /// Attribute defaults of a `type` alias: `(start = 0, fixed = true)`,
    /// plus the `unit` string if one is named.
    pub(super) fn type_attributes(&mut self) -> Result<AliasAttributes, ParseError> {
        self.expect(&Token::LParen, "type attributes")?;
        let mut out = Vec::new();
        let mut unit = None;
        loop {
            let name = self.ident("attribute name")?;
            self.expect(&Token::Assign, "`=` in a type attribute")?;
            // The unit string feeds the dimensional check; the other
            // descriptive attributes are kept as opaque text and
            // ignored by the compiler.
            let value = match self.peek().clone() {
                Token::Str(text) => {
                    self.bump();
                    if name == "unit" {
                        unit = Some(text);
                    }
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
        Ok((out, unit))
    }

    /// A class-level `annotation ( ... ) ;`.
    pub(super) fn parse_annotation(
        &mut self,
        experiment: &mut Experiment,
    ) -> Result<(), ParseError> {
        self.annotation_body(experiment)?;
        self.expect(&Token::Semi, "semicolon after annotation")?;
        Ok(())
    }

    /// `annotation ( ... )` without its terminator — declarations,
    /// equations and `connect` statements carry one before the
    /// semicolon. Parsed tolerantly: only
    /// `experiment(StopTime=…, Interval=…, Tolerance=…)` is extracted,
    /// everything else is skipped by balancing parentheses.
    pub(super) fn annotation_body(
        &mut self,
        experiment: &mut Experiment,
    ) -> Result<(), ParseError> {
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
}
