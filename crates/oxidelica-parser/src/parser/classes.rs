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
                Token::Model | Token::Class => ClassKind::Model,
                Token::Block => ClassKind::Block,
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
        // `redeclare record extends SaturationProperties ... end
        // SaturationProperties;` - a class that replaces one it
        // inherited by extending it. Its name is the name it extends,
        // and what it adds is written in its body like any other
        // class's.
        if self.peek() == &Token::Extends {
            self.bump();
            let name = self.dotted_name("the class being extended")?;
            let (modifiers, redeclares, _each, broken) = if self.peek() == &Token::LParen {
                self.modifier_list()?
            } else {
                (Vec::new(), Vec::new(), Vec::new(), Vec::new())
            };
            let inherited = Extend {
                base: name.clone(),
                modifiers,
                broken,
                redeclares,
                from_base: true,
            };
            return self.class_body(
                kind,
                name,
                partial,
                encapsulated,
                expandable,
                vec![inherited],
            );
        }

        let name = self.ident("class name")?;

        // `package Medium = Media.Water constrainedby PartialMedium;` -
        // a short class definition: the enclosing class gets a local
        // name for another class, replaceable when marked so.
        if kind != ClassKind::Type && self.peek() == &Token::Assign {
            self.bump();
            // `connector RealInput = input Real "'input Real' as
            // connector";` - a short definition may repeat the prefixes
            // of a declaration. They restrict what the name may be used
            // for and change nothing about the class itself, save that
            // `input` or `output` is what makes a signal connector a
            // causal one, which is what a `block` is allowed to hold.
            let mut alias_causality = Causality::None;
            while matches!(
                self.peek(),
                Token::Input
                    | Token::Output
                    | Token::Flow
                    | Token::Stream
                    | Token::Discrete
                    | Token::Parameter
                    | Token::Constant
            ) {
                alias_causality = match self.bump() {
                    Token::Input => Causality::Input,
                    Token::Output => Causality::Output,
                    _ => alias_causality,
                };
            }
            let target = self.dotted_name("aliased class")?;
            // A short definition of a predefined type declares a type
            // of its own rather than a second name for a class: the
            // standard library's signal connectors are `connector
            // RealInput = input Real`. Recorded the way `type` is, so
            // that a component declared with one resolves down to the
            // primitive it holds.
            if is_predefined(&target) {
                let alias_dimensions = self.type_dimensions()?;
                let (attributes, unit) = if self.peek() == &Token::LParen {
                    self.type_attributes()?
                } else {
                    (Vec::new(), None)
                };
                self.opt_string();
                if self.peek() == &Token::Annotation {
                    self.annotation_body(&mut Annotated::default())?;
                }
                self.expect(&Token::Semi, "semicolon after the class alias")?;
                return Ok(ClassItem::Class(Box::new(ClassDef {
                    kind,
                    name,
                    partial,
                    encapsulated,
                    expandable,
                    alias_of: Some((target, attributes)),
                    alias_dimensions,
                    alias_unit: unit,
                    alias_causality,
                    ..ClassDef::empty()
                })));
            }
            // Modifiers on the target are kept: `redeclare function f
            // = g(a = 1)` fills in some of a function's inputs and
            // hands the rest over, which is the same partial
            // application as `function g(a = 1)` written where a
            // declaration goes. The pumps of the standard library ask
            // for their characteristics this way.
            let mut modifiers = Vec::new();
            if self.peek() == &Token::LParen {
                modifiers = self.modifier_list()?.0;
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
                self.annotation_body(&mut Annotated::default())?;
            }
            self.expect(&Token::Semi, "semicolon after the class alias")?;
            return Ok(ClassItem::Alias(ClassAlias {
                name,
                target,
                replaceable,
                redeclaration,
                constrained_by,
                modifiers,
                connector: kind == ClassKind::Connector,
                causality: alias_causality,
            }));
        }

        // `type Voltage = Real(start = 0);` or
        // `type Init = enumeration(NoInit, SteadyState);`
        let mut alias_of = None;
        let mut alias_dimensions = Vec::new();
        let mut alias_unit = None;
        let mut enumeration = Vec::new();
        // A `type` is usually the short form, `type Voltage = Real(...)`,
        // but the long one is a class body like any other - the standard
        // library writes its icons that way, `type TypeReal "..." extends
        // Real; annotation(...); end TypeReal;`. Only the short form is
        // handled here; without the `=` it falls through to the body
        // below.
        if kind == ClassKind::Type && self.peek() == &Token::Assign {
            self.bump();
            if self.peek() == &Token::Enumeration {
                enumeration = self.enumeration_literals()?;
            } else {
                let base = self.dotted_name("aliased type")?;
                alias_dimensions = self.type_dimensions()?;
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
                self.annotation_body(&mut Annotated::default())?;
            }
            self.expect(&Token::Semi, "semicolon after the type alias")?;
            return Ok(ClassItem::Class(Box::new(ClassDef {
                kind,
                name,
                partial,
                encapsulated,
                expandable,
                alias_of,
                alias_dimensions,
                alias_unit,
                enumeration,
                ..ClassDef::empty()
            })));
        }

        self.class_body(kind, name, partial, encapsulated, expandable, Vec::new())
    }

    /// The body of a class: what stands between its name and its `end`.
    ///
    /// `inherited` carries the `extends` a class written as
    /// `redeclare record extends X` already has - its own body may add
    /// more.
    #[allow(clippy::too_many_arguments)]
    fn class_body(
        &mut self,
        kind: ClassKind,
        name: String,
        partial: bool,
        encapsulated: bool,
        expandable: bool,
        inherited: Vec<Extend>,
    ) -> Result<ClassItem, ParseError> {
        let mut alias_of: Option<(String, Vec<(String, Expr)>)> = None;
        let alias_dimensions: Vec<Expr> = Vec::new();
        let alias_unit: Option<String> = None;
        let enumeration: Vec<String> = Vec::new();
        let description = self.opt_string();

        let mut nested = Vec::new();
        let mut class_aliases = Vec::new();
        let mut imports = Vec::new();
        let mut components = Vec::new();
        let mut extends = inherited;
        let mut equations = Vec::new();
        let mut connects = Vec::new();
        let mut when_clauses = Vec::new();
        let mut for_equations = Vec::new();
        let mut if_equations = Vec::new();
        let mut asserts = Vec::new();
        let mut calls = Vec::new();
        let mut transitions = Vec::new();
        let mut initial_state = None;
        let mut connection_graph = Vec::new();
        let mut algorithm = Vec::new();
        let mut initial_algorithm = Vec::new();
        let mut external = false;
        let mut external_call = None;
        let mut builtin = None;
        let mut initial_equations = Vec::new();
        let mut annotated = Annotated::default();
        let mut in_equations = false;
        // A `protected` heading holds until a `public` one takes over,
        // so what a declaration is reachable from is decided by which
        // of the two was seen last.
        let mut in_protected = false;
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
                Token::Initial if self.peek_ahead(1) == &Token::Algorithm => {
                    self.bump();
                    self.bump();
                    initial_algorithm.extend(self.statements()?);
                }
                Token::Algorithm => {
                    self.bump();
                    algorithm.extend(self.statements()?);
                }
                Token::For => {
                    for_equations.push(self.for_equation()?);
                }
                // An `if` written in an `initial equation` section
                // says where the run begins rather than what holds
                // throughout it, and what its branch holds has to join
                // the initial equations. Read as an ordinary `if`, the
                // branch became an equation of the running model and
                // the model came out with one equation more than it
                // had unknowns.
                Token::If if in_equations => {
                    let mut written = self.if_equation()?;
                    written.initial = in_initial;
                    if_equations.push(written);
                }
                Token::Annotation => {
                    self.parse_annotation(&mut annotated)?;
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
                    in_protected = self.bump() == Token::Protected;
                    // A section heading also ends an equation section:
                    // what follows it is declarations again.
                    in_equations = false;
                    in_initial = false;
                }
                // An implementation outside Modelica. The class is read
                // so that the file it shares with others still loads;
                // what cannot be done is to run it, and that is said
                // where such a function is called.
                Token::External => {
                    let named = self.external_body()?;
                    // `external "builtin" y = asin(u)` says the
                    // function is an operator the language already has,
                    // given a place in a library's tree.
                    builtin = named
                        .as_ref()
                        .filter(|call| call.language.as_deref() == Some("builtin"))
                        .map(|call| call.called.clone());
                    external_call = named;
                    external = true;
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
                    ClassItem::Class(mut class) => {
                        class.protected = in_protected;
                        nested.push(*class);
                    }
                    ClassItem::Alias(alias) => class_aliases.push(alias),
                },
                // `replaceable`/`redeclare` introduce either a nested
                // class or a component, and they may come together -
                // `redeclare replaceable model extends B` - so what
                // decides is the first word past the prefixes.
                Token::Replaceable | Token::Redeclare if self.class_ahead() => {
                    match self.class_def()? {
                        ClassItem::Class(mut class) => {
                            class.protected = in_protected;
                            nested.push(*class);
                        }
                        ClassItem::Alias(alias) => class_aliases.push(alias),
                    }
                }
                Token::Eof => return Err(self.err("unexpected end of file: missing end".into())),
                // `assert(condition, "message")` is a runtime check.
                Token::Ident(name) if in_equations && name == "assert" => {
                    self.bump();
                    let held = self.assert_arguments()?;
                    if self.peek() == &Token::Annotation {
                        self.annotation_body(&mut Annotated::default())?;
                    }
                    self.expect(&Token::Semi, "semicolon after assert")?;
                    asserts.extend(held);
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
                        self.annotation_body(&mut Annotated::default())?;
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
                        match self.equation_line()? {
                            EquationLine::Equation(equation) => initial_equations.push(equation),
                            EquationLine::Call(call) => calls.push(call),
                        }
                    } else if in_equations {
                        match self.equation_line()? {
                            EquationLine::Equation(equation) => equations.push(equation),
                            EquationLine::Call(call) => calls.push(call),
                        }
                    } else {
                        components.extend(self.declaration()?.into_iter().map(|mut one| {
                            one.protected = in_protected;
                            one
                        }));
                    }
                }
            }
        }

        // The long form of a type extends the type it is built on -
        // `type TypeString extends String; ... end TypeString;`, and
        // `type Orientation extends TransformationMatrix; ... end
        // Orientation;` where that one is `Real[3, 3]`. A type can
        // inherit nothing from a type but the type itself, so this is
        // the short form written out, and it is recorded as one.
        if alias_of.is_none()
            && extends.len() == 1
            && (is_predefined(&extends[0].base)
                || (kind == ClassKind::Type && components.is_empty()))
        {
            let base = extends.remove(0);
            alias_of = Some((base.base, base.modifiers));
        }

        Ok(ClassItem::Class(Box::new(ClassDef {
            kind,
            name,
            partial,
            encapsulated,
            protected: false,
            expandable,
            alias_of,
            alias_causality: Causality::None,
            alias_dimensions,
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
            initial_algorithm,
            external,
            external_call,
            builtin,
            connects,
            when_clauses,
            experiment: annotated.experiment,
            derivative: annotated.derivative,
            derivative_needs_still: annotated.derivative_needs_still,
            inverse: annotated.inverse,
            annotations: annotated.kept,
            class_aliases,
            asserts,
            calls,
            transitions,
            initial_state,
            connection_graph,
        })))
    }

    /// Whether what starts here is a class rather than a component.
    /// The prefixes a class and a component share are stepped over -
    /// `redeclare replaceable model` is three words before the one
    /// that says which.
    fn class_ahead(&self) -> bool {
        let mut ahead = 0;
        while matches!(
            self.peek_ahead(ahead),
            Token::Replaceable
                | Token::Redeclare
                | Token::Final
                | Token::Inner
                | Token::Outer
                | Token::Each
        ) {
            ahead += 1;
        }
        matches!(
            self.peek_ahead(ahead),
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
        )
    }

    /// The end of an `import`: a description of it, an annotation on
    /// it, and the semicolon. The standard library says what an import
    /// is for - `import Medium = ...Air_pT "Medium model";` - and a
    /// description belongs to whoever reads the source rather than to
    /// anything here.
    fn end_of_import(&mut self) -> Result<(), ParseError> {
        if matches!(self.peek(), Token::Str(_)) {
            self.bump();
        }
        if self.peek() == &Token::Annotation {
            self.annotation_body(&mut Annotated::default())?;
        }
        self.expect(&Token::Semi, "semicolon after import")?;
        Ok(())
    }

    /// `import A.B.C;` or `import D = A.B.C;`
    pub(super) fn import_clause(&mut self) -> Result<Vec<(String, String)>, ParseError> {
        self.expect(&Token::Import, "import")?;
        let first = self.ident("imported name")?;
        if self.peek() == &Token::Assign {
            self.bump();
            let target = self.dotted_name("import target")?;
            self.end_of_import()?;
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
                self.end_of_import()?;
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
            self.end_of_import()?;
            return Ok(vec![(WILDCARD_IMPORT.to_string(), target)]);
        }
        self.end_of_import()?;
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
        self.opt_string();
        if self.peek() == &Token::Annotation {
            self.annotation_body(&mut Annotated::default())?;
        }
        self.expect(&Token::Semi, "semicolon after extends")?;
        Ok(Extend {
            base,
            modifiers,
            broken,
            redeclares,
            from_base: false,
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
            // `type Angle = Real(final unit = "rad")` - `final` forbids
            // anyone downstream from setting the attribute again, and
            // `each` spreads one value over an array. Neither changes
            // the value, so both are read and dropped.
            while matches!(self.peek(), Token::Final | Token::Each) {
                self.bump();
            }
            let name = self.ident("attribute name")?;
            self.expect(&Token::Assign, "`=` in a type attribute")?;
            // The unit string feeds the dimensional check; the other
            // descriptive attributes are kept as opaque text and
            // ignored by the compiler.
            let value = match self.peek().clone() {
                // A descriptive attribute may be worked out rather than
                // written: the standard library builds a quantity out
                // of the name of the medium. Only one written as a
                // plain string is taken as one.
                Token::Str(text) if matches!(self.peek_at(1), Token::Comma | Token::RParen) => {
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
    pub(super) fn parse_annotation(&mut self, into: &mut Annotated) -> Result<(), ParseError> {
        self.annotation_body(into)?;
        self.expect(&Token::Semi, "semicolon after annotation")?;
        Ok(())
    }

    /// `annotation ( ... )` without its terminator — declarations,
    /// equations and `connect` statements carry one before the
    /// semicolon. Parsed tolerantly: only
    /// `experiment(StopTime=…, Interval=…, Tolerance=…)` is extracted,
    /// everything else is skipped by balancing parentheses.
    pub(super) fn annotation_body(&mut self, into: &mut Annotated) -> Result<(), ParseError> {
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
                // `derivative = f_der` names the function that gives
                // this one's derivative. The options the specification
                // allows alongside it - an order, a `noDerivative` or a
                // `zeroDerivative` - change which arguments the named
                // function takes and what it answers. Reading them
                // wrong would give a wrong derivative, which nothing
                // downstream could catch, so an annotation carrying any
                // of them is read past and not kept: a function with no
                // derivative anyone here can use is one whose
                // derivative is refused where it is asked for, which
                // says so instead of being quietly wrong.
                Token::Ident(name) if depth == 1 && name == "derivative" => {
                    self.bump();
                    if self.peek() == &Token::LParen {
                        // `derivative(zeroDerivative = delta) = f_der`
                        // says the rule holds wherever `delta` does not
                        // change with time. That is how the fluid
                        // library writes its smoothing functions: the
                        // regularisation width is a parameter, so the
                        // rule applies and the body - which has an
                        // `abs` in it - never has to be differentiated.
                        //
                        // Anything else allowed here changes what the
                        // named function takes or answers, and reading
                        // that wrong is a wrong derivative nothing
                        // downstream could catch. An order or a
                        // `noDerivative` is still read past.
                        let mut needs_still = Vec::new();
                        let mut only_zero_derivatives = true;
                        let mut inner = 0usize;
                        loop {
                            match self.bump() {
                                Token::LParen => inner += 1,
                                Token::RParen => {
                                    inner -= 1;
                                    if inner == 0 {
                                        break;
                                    }
                                }
                                Token::Ident(option) if inner == 1 => {
                                    if option == "zeroDerivative" {
                                        if self.peek() == &Token::Assign {
                                            self.bump();
                                            needs_still.push(self.ident("the input held still")?);
                                        }
                                    } else {
                                        only_zero_derivatives = false;
                                    }
                                }
                                Token::Eof => {
                                    return Err(self.err("unterminated derivative options".into()))
                                }
                                _ => {}
                            }
                        }
                        self.expect(&Token::Assign, "`=` after derivative")?;
                        let named = self.dotted_name("the derivative function")?;
                        if only_zero_derivatives && !needs_still.is_empty() {
                            into.derivative = Some(named);
                            into.derivative_needs_still = needs_still;
                        }
                        continue;
                    }
                    self.expect(&Token::Assign, "`=` after derivative")?;
                    into.derivative = Some(self.dotted_name("the derivative function")?);
                }
                // `inverse(x = f_inv(y, z))` says this function can be
                // solved for `x` by calling `f_inv`.
                Token::Ident(name) if depth == 1 && name == "inverse" => {
                    self.bump();
                    self.expect(&Token::LParen, "parenthesis after inverse")?;
                    loop {
                        let solved_for = self.ident("the input the inverse solves for")?;
                        self.expect(&Token::Assign, "`=` in inverse")?;
                        let called = self.dotted_name("the inverse function")?;
                        self.expect(&Token::LParen, "parenthesis after the inverse function")?;
                        let mut arguments = Vec::new();
                        while self.peek() != &Token::RParen {
                            // The argument may be named the way any
                            // call's may - `h_pTX(p=p, T=T, X=X)` is
                            // how the moist air tables write theirs -
                            // and what the inverse needs is which
                            // input it stands for, which is the name
                            // on the right either way.
                            let written = self.ident("an argument of the inverse")?;
                            if self.peek() == &Token::Assign {
                                self.bump();
                                arguments.push(self.ident("the value of a named argument")?);
                            } else {
                                arguments.push(written);
                            }
                            if self.peek() == &Token::Comma {
                                self.bump();
                            }
                        }
                        self.bump();
                        into.inverse.push((solved_for, called, arguments));
                        match self.bump() {
                            Token::Comma => continue,
                            Token::RParen => break,
                            other => {
                                return Err(self.err(format!(
                                    "expected `,` or `)` in inverse, found `{other}`"
                                )))
                            }
                        }
                    }
                }
                Token::Ident(name) if depth == 1 && name == "experiment" => {
                    self.bump();
                    self.expect(&Token::LParen, "parenthesis after experiment")?;
                    loop {
                        let key = self.ident("experiment parameter")?;
                        self.expect(&Token::Assign, "`=` in experiment")?;
                        let value = self.number_literal()?;
                        match key.as_str() {
                            "StopTime" => into.experiment.stop_time = Some(value),
                            "Interval" => into.experiment.interval = Some(value),
                            "Tolerance" => into.experiment.tolerance = Some(value),
                            "StartTime" => into.experiment.start_time = Some(value),
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
                // Anything else is read as what it is - `Icon(graphics
                // = {Line(points = {{0, 0}})})` is a call with named
                // arguments, which the expression parser already knows.
                // What it cannot read is skipped rather than refused:
                // an annotation says things to tools, and a tool that
                // does not understand one has to carry on regardless.
                Token::Ident(_) if depth == 1 => {
                    let saved = self.pos;
                    match self.annotation_entry() {
                        Ok(entry) => into.kept.push(entry),
                        Err(_) => {
                            self.pos = saved;
                            self.skip_entry(&mut depth);
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

    /// One `name`, `name = value` or `name(...)` of an annotation.
    fn annotation_entry(&mut self) -> Result<Expr, ParseError> {
        let name = self.dotted_name("the name of an annotation")?;
        match self.peek() {
            Token::Assign => {
                self.bump();
                let value = self.expr()?;
                Ok(Expr::NamedArg(name, Box::new(value)))
            }
            Token::LParen => {
                self.pos -= 1;
                let Expr::Call(called, args) = self.primary()? else {
                    return Err(self.err(format!("`{name}` is not an annotation this reads")));
                };
                Ok(Expr::Call(called, args))
            }
            // A bare word says something by being there at all.
            _ => Ok(Expr::Ref(name)),
        }
    }

    /// `[4]` after the type a short definition names: a type that is
    /// an array rather than a value.
    fn type_dimensions(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut dimensions = Vec::new();
        if self.peek() != &Token::LBracket {
            return Ok(dimensions);
        }
        self.bump();
        loop {
            dimensions.push(self.subscript()?);
            match self.bump() {
                Token::Comma => continue,
                Token::RBracket => break,
                other => {
                    return Err(self.err(format!(
                        "expected `,` or `]` in the dimensions of a type, found `{other}`"
                    )))
                }
            }
        }
        Ok(dimensions)
    }

    /// `external "C" y = f(x) annotation(Library = "m");` — the
    /// language, the call and the annotation that says where to find
    /// the object code.
    ///
    /// The one language this compiler can honour is `"builtin"`, which
    /// says the function is an operator the language already has:
    /// `external "builtin" y = asin(u);` is how the standard library
    /// gives `asin` a place in its tree. The operator's name comes back
    /// where that is what was written. Everything else is read only far
    /// enough to get past it, and refused where such a function is
    /// called.
    fn external_body(&mut self) -> Result<Option<ExternalCall>, ParseError> {
        self.expect(&Token::External, "external")?;
        let language = self.opt_string();
        // `y = asin(u)` or `asin(u)`: a name, and the name of what it
        // calls if the result is assigned.
        let (mut answer, mut called) = (None, None);
        if matches!(self.peek(), Token::Ident(_)) {
            let first = self.ident("the external function or its result")?;
            called = Some(match self.peek() {
                Token::Assign => {
                    self.bump();
                    answer = Some(first);
                    self.ident("the external function")?
                }
                _ => first,
            });
        }
        // What it is handed, as written. A declaration this compiler
        // cannot read the arguments of is still a declaration it can
        // read past, so the reading is a try: what it does not manage
        // is left as nothing, and the clause is stepped over as before.
        let mut arguments = Vec::new();
        if called.is_some() && self.peek() == &Token::LParen {
            let saved = self.pos;
            self.bump();
            loop {
                if self.peek() == &Token::RParen {
                    self.bump();
                    break;
                }
                match self.expr() {
                    Ok(argument) => arguments.push(argument),
                    Err(_) => {
                        arguments.clear();
                        self.pos = saved;
                        break;
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
                    _ => {
                        arguments.clear();
                        self.pos = saved;
                        break;
                    }
                }
            }
        }
        let mut depth = 0usize;
        loop {
            match self.peek() {
                Token::Eof => return Err(self.err("unterminated external clause".into())),
                Token::Semi if depth == 0 => {
                    self.bump();
                    break;
                }
                Token::LParen | Token::LBrace | Token::LBracket => {
                    depth += 1;
                    self.bump();
                }
                Token::RParen | Token::RBrace | Token::RBracket => {
                    depth = depth.saturating_sub(1);
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
        Ok(called.map(|called| ExternalCall {
            language,
            answer,
            called,
            arguments,
        }))
    }

    /// Step over one entry of an annotation, whatever it is made of.
    fn skip_entry(&mut self, depth: &mut usize) {
        let outside = *depth;
        loop {
            match self.peek() {
                Token::Eof => return,
                Token::LParen | Token::LBrace | Token::LBracket => {
                    *depth += 1;
                    self.bump();
                }
                Token::RParen | Token::RBrace | Token::RBracket => {
                    if *depth == outside {
                        return;
                    }
                    *depth -= 1;
                    self.bump();
                }
                Token::Comma if *depth == outside => return,
                _ => {
                    self.bump();
                }
            }
        }
    }
}

/// The types the language itself defines. A short class definition
/// naming one of them - `connector RealInput = input Real` - is a type
/// of its own rather than another name for a class.
fn is_predefined(name: &str) -> bool {
    matches!(name, "Real" | "Integer" | "Boolean" | "String" | "Clock")
}
