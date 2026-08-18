//! Component declarations and the modifiers that reach into them.

use super::*;

impl Parser {
    pub(super) fn declaration(&mut self) -> Result<Component, ParseError> {
        // Declaration prefixes may come in any order the specification
        // allows: `inner replaceable parameter Real k`.
        let mut variability = Variability::Continuous;
        let mut flow = false;
        let mut stream = false;
        let mut causality = Causality::None;
        let mut scope = Scope::Local;
        let mut replaceable = false;
        let mut redeclaration = false;
        let mut is_final = false;
        loop {
            match self.peek() {
                Token::Parameter => variability = Variability::Parameter,
                Token::Constant => variability = Variability::Constant,
                Token::Discrete => variability = Variability::Discrete,
                Token::Flow => flow = true,
                Token::Stream => stream = true,
                Token::Input => causality = Causality::Input,
                Token::Output => causality = Causality::Output,
                Token::Inner => scope = Scope::Inner,
                // `inner outer x` owns the instance and refers to the
                // enclosing one; owning is what creates the variables.
                Token::Outer if scope != Scope::Inner => scope = Scope::Outer,
                Token::Outer => {}
                Token::Replaceable => replaceable = true,
                Token::Redeclare => redeclaration = true,
                Token::Final => is_final = true,
                Token::Each => {}
                _ => break,
            }
            self.bump();
        }

        let type_name = self.dotted_name("component type")?;
        let name = self.ident("component name")?;
        // `Real T[N, 3]` — dimensions are constant expressions, except
        // for `Real v[:]`, where a colon leaves the length to whatever
        // the argument at the call site turns out to be.
        let mut dimensions = Vec::new();
        if self.peek() == &Token::LBracket {
            self.bump();
            loop {
                dimensions.push(self.subscript()?);
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
        let mut unit = None;
        let (mut min, mut max) = (None, None);
        let mut modifiers = Vec::new();
        let mut redeclares = Vec::new();
        let mut each_modifiers = Vec::new();
        if self.peek() == &Token::LParen {
            if matches!(
                type_name.as_str(),
                "Real" | "Integer" | "Boolean" | "String"
            ) {
                self.bump();
                loop {
                    while matches!(self.peek(), Token::Final | Token::Each) {
                        self.bump();
                    }
                    let attr = self.ident("attribute name")?;
                    self.expect(&Token::Assign, "`=` in attribute")?;
                    match attr.as_str() {
                        "start" => start = Some(self.expr()?),
                        "min" => min = Some(self.expr()?),
                        "max" => max = Some(self.expr()?),
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
                        "unit" => match self.bump() {
                            Token::Str(text) => unit = Some(text),
                            other => {
                                return Err(
                                    self.err(format!("unit expects a string, found `{other}`"))
                                )
                            }
                        },
                        // The remaining attributes (nominal, quantity,
                        // stateSelect, …) describe the variable rather
                        // than the equations: parsed and dropped.
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
                (modifiers, redeclares, each_modifiers, _) = self.modifier_list()?;
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
            stream,
            dimensions,
            causality,
            modifiers,
            variability,
            start,
            fixed,
            unit,
            min,
            max,
            binding,
            description,
            scope,
            replaceable,
            constrained_by,
            condition,
            redeclares,
            redeclaration,
            is_final,
            each_modifiers,
        })
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
    pub(super) fn modifier_list(&mut self) -> Result<Modifications, ParseError> {
        self.expect(&Token::LParen, "modifier list")?;
        let mut modifiers = Vec::new();
        let mut redeclares = Vec::new();
        let mut each_names = Vec::new();
        let mut broken = Vec::new();
        // An empty list, `Interface()`, modifies nothing.
        if self.peek() == &Token::RParen {
            self.bump();
            return Ok((modifiers, redeclares, each_names, broken));
        }
        loop {
            let mut has_each = false;
            while matches!(self.peek(), Token::Final | Token::Each) {
                has_each |= self.peek() == &Token::Each;
                self.bump();
            }
            if self.peek() == &Token::Break {
                // Selective extension: `break f` leaves out a component
                // of the base, `break connect(a, b)` one connection.
                self.bump();
                if self.peek() == &Token::Connect {
                    self.bump();
                    self.expect(&Token::LParen, "`(` after break connect")?;
                    let a = self.component_ref()?;
                    self.expect(&Token::Comma, "`,` in break connect")?;
                    let b = self.component_ref()?;
                    self.expect(&Token::RParen, "`)` after break connect")?;
                    broken.push(Deselect::Connection(a, b));
                } else {
                    broken.push(Deselect::Component(self.component_ref()?));
                }
            } else if self.peek() == &Token::Redeclare {
                redeclares.push(self.redeclaration()?);
            } else {
                let name = self.component_ref()?;
                if has_each {
                    each_names.push(name.clone());
                }
                if self.peek() == &Token::LParen {
                    let (nested, nested_redeclares, nested_each, _nested_break) =
                        self.modifier_list()?;
                    modifiers.extend(
                        nested
                            .into_iter()
                            .map(|(sub, value)| (format!("{name}.{sub}"), value)),
                    );
                    each_names.extend(nested_each.into_iter().map(|sub| format!("{name}.{sub}")));
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
        Ok((modifiers, redeclares, each_names, broken))
    }

    /// Whether the current token closes a modifier or the whole list.
    pub(super) fn at_modifier_end(&self) -> bool {
        matches!(self.peek(), Token::Comma | Token::RParen)
    }

    /// The value of one modifier. `None` means the value was a string:
    /// the compiler has no use for it, so the modifier is dropped.
    pub(super) fn modifier_value(&mut self) -> Result<Option<Expr>, ParseError> {
        if matches!(self.peek(), Token::Str(_)) {
            self.bump();
            return Ok(None);
        }
        Ok(Some(self.expr()?))
    }

    /// `redeclare [replaceable] Type name(modifiers) [constrainedby C]`
    /// inside a modifier list.
    pub(super) fn redeclaration(&mut self) -> Result<Redeclare, ParseError> {
        self.expect(&Token::Redeclare, "redeclare")?;
        while matches!(self.peek(), Token::Replaceable | Token::Final | Token::Each) {
            self.bump();
        }
        // `redeclare package Medium = Oil` swaps a class alias.
        if matches!(
            self.peek(),
            Token::Package
                | Token::Model
                | Token::Block
                | Token::Function
                | Token::Record
                | Token::Connector
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

    /// A dotted component reference: `a.b.c`.
    pub(super) fn component_ref(&mut self) -> Result<String, ParseError> {
        let mut name = self.ident("component reference")?;
        while self.peek() == &Token::Dot {
            self.bump();
            name.push('.');
            name.push_str(&self.ident("name after dot")?);
        }
        Ok(name)
    }
}
