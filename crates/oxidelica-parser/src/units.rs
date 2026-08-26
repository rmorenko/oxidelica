//! Units, as they are written and as they are compared.
//!
//! A unit string - `"N.m"`, `"m/s2"`, `"J/(kg.K)"` - is read into
//! exponents over the seven base dimensions. Scale factors are
//! dropped, since consistency is what is being checked and `g` and
//! `kg` are the same dimension; angles count as dimensionless, which
//! is what lets `tau = J * der(w)` hold with torque in N.m.
//!
//! Carved out of `check` unchanged.

/// Angles (`rad`, `sr`, `deg`) count as dimensionless, which is what
/// lets `tau = J * der(w)` hold with torque in N.m.
pub(crate) const BASES: usize = 7;
pub(crate) const BASE_NAMES: [&str; BASES] = ["m", "kg", "s", "A", "K", "mol", "cd"];

/// Exponents over the base dimensions; scale factors are irrelevant
/// for consistency, so `g` and `kg` are the same dimension.
///
/// The exponents are rational, not whole: the grammar writes `m(1/2)`,
/// and the square root of an odd power lands there too. One shared
/// denominator across the bases is enough, and keeping it reduced makes
/// two equal dimensions equal field by field, which is what the derived
/// `Eq` needs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Dim {
    pub(crate) powers: [i32; BASES],
    pub(crate) denominator: i32,
}

impl Dim {
    pub(crate) const ONE: Dim = Dim {
        powers: [0; BASES],
        denominator: 1,
    };

    /// Whole-number exponents, the common case: base and derived units.
    pub(crate) fn of(powers: [i32; BASES]) -> Dim {
        Dim {
            powers,
            denominator: 1,
        }
    }

    /// Reduce to the canonical form: a positive denominator, and no
    /// factor shared by it and every power.
    pub(crate) fn reduced(mut powers: [i32; BASES], mut denominator: i32) -> Dim {
        if denominator < 0 {
            denominator = -denominator;
            powers = powers.map(|power| -power);
        }
        let mut divisor = denominator;
        for &power in &powers {
            divisor = gcd(divisor, power);
        }
        if divisor == 0 {
            divisor = 1;
        }
        Dim {
            powers: powers.map(|power| power / divisor),
            denominator: denominator / divisor,
        }
    }

    /// Multiply every exponent by the rational `n / d`: a power, a root,
    /// or the sign of a division.
    pub(crate) fn scaled(self, n: i32, d: i32) -> Dim {
        Dim::reduced(self.powers.map(|power| power * n), self.denominator * d)
    }

    /// `self` times `other^sign`, over a common denominator.
    pub(crate) fn combine(self, other: Dim, sign: i32) -> Dim {
        let denominator = lcm(self.denominator, other.denominator);
        let mine = denominator / self.denominator;
        let theirs = denominator / other.denominator;
        let mut powers = [0; BASES];
        for (index, slot) in powers.iter_mut().enumerate() {
            *slot = self.powers[index] * mine + sign * other.powers[index] * theirs;
        }
        Dim::reduced(powers, denominator)
    }

    /// `m2.kg.s-3.A-1`, or `m(1/2)` where an exponent is not whole - the
    /// canonical spelling for error messages.
    pub(crate) fn text(self) -> String {
        let parts: Vec<String> = self
            .powers
            .iter()
            .zip(BASE_NAMES)
            .filter(|(&power, _)| power != 0)
            .map(|(&power, name)| {
                let divisor = gcd(power, self.denominator);
                let (whole, over) = (power / divisor, self.denominator / divisor);
                match (whole, over) {
                    (1, 1) => name.to_string(),
                    (_, 1) => format!("{name}{whole}"),
                    _ => format!("{name}({whole}/{over})"),
                }
            })
            .collect();
        if parts.is_empty() {
            "1".to_string()
        } else {
            parts.join(".")
        }
    }
}

/// Greatest common divisor, for keeping a dimension reduced.
fn gcd(a: i32, b: i32) -> i32 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Least common multiple of two denominators, both positive.
fn lcm(a: i32, b: i32) -> i32 {
    if a == 0 || b == 0 {
        1
    } else {
        a / gcd(a, b) * b
    }
}

/// Parse a Modelica unit string (`"N.m"`, `"m/s2"`, `"J/(kg.K)"`) into
/// dimensions. `None` means a symbol this table does not know; the
/// whole unit then carries no information rather than a wrong one.
pub(crate) fn parse_unit(text: &str) -> Option<Dim> {
    let mut reader = UnitReader {
        chars: text.chars().collect(),
        at: 0,
    };
    let dim = reader.expression()?;
    if reader.at == reader.chars.len() {
        Some(dim)
    } else {
        None
    }
}

struct UnitReader {
    chars: Vec<char>,
    at: usize,
}

impl UnitReader {
    /// `numerator [ "/" denominator ]`: one division at most, which is
    /// what the grammar allows - `N.m/s/K` is not a unit, `N.m/(s.K)`
    /// is. A denominator of several factors has to be parenthesised.
    pub(crate) fn expression(&mut self) -> Option<Dim> {
        let mut dim = self.numerator()?;
        if self.chars.get(self.at) == Some(&'/') {
            self.at += 1;
            dim = dim.combine(self.denominator()?, -1);
        }
        Some(dim)
    }

    /// `"1" | factors | "(" expression ")"`.
    pub(crate) fn numerator(&mut self) -> Option<Dim> {
        if self.chars.get(self.at) == Some(&'(') {
            return self.group();
        }
        let mut dim = self.factor()?;
        while self.chars.get(self.at) == Some(&'.') {
            self.at += 1;
            dim = dim.combine(self.factor()?, 1);
        }
        Some(dim)
    }

    /// `factor | "(" expression ")"`: a single factor, unless grouped.
    pub(crate) fn denominator(&mut self) -> Option<Dim> {
        if self.chars.get(self.at) == Some(&'(') {
            return self.group();
        }
        self.factor()
    }

    /// `"(" expression ")"`.
    pub(crate) fn group(&mut self) -> Option<Dim> {
        self.at += 1;
        let inner = self.expression()?;
        if self.chars.get(self.at) != Some(&')') {
            return None;
        }
        self.at += 1;
        Some(inner)
    }

    /// One symbol with an optional exponent: `m`, `s-1`, `m(1/2)`.
    pub(crate) fn factor(&mut self) -> Option<Dim> {
        let start = self.at;
        while self
            .chars
            .get(self.at)
            .is_some_and(|c| c.is_alphabetic() || *c == '%')
        {
            self.at += 1;
        }
        // A bare "1" is the dimensionless unit; any other digit here
        // belongs to an exponent and stays for the code below.
        if start == self.at && self.chars.get(self.at) == Some(&'1') {
            self.at += 1;
            return Some(Dim::ONE);
        }
        let symbol: String = self.chars[start..self.at].iter().collect();
        if symbol.is_empty() {
            return None;
        }
        let base = symbol_dimensions(&symbol)?;
        let (whole, over) = self.exponent()?;
        Some(base.scaled(whole, over))
    }

    /// The exponent of a factor: absent (so `1/1`), a signed integer
    /// (`m2`, `s-1`), or a signed rational in parentheses (`m(1/2)`).
    pub(crate) fn exponent(&mut self) -> Option<(i32, i32)> {
        let before = self.at;
        let sign = match self.chars.get(self.at) {
            Some('+') => {
                self.at += 1;
                1
            }
            Some('-') => {
                self.at += 1;
                -1
            }
            _ => 1,
        };
        if self.chars.get(self.at) == Some(&'(') {
            self.at += 1;
            let numerator = self.unsigned()?;
            if self.chars.get(self.at) != Some(&'/') {
                return None;
            }
            self.at += 1;
            let denominator = self.unsigned()?;
            if denominator == 0 || self.chars.get(self.at) != Some(&')') {
                return None;
            }
            self.at += 1;
            return Some((sign * numerator, denominator));
        }
        if let Some(whole) = self.unsigned() {
            return Some((sign * whole, 1));
        }
        // A sign with no number behind it was never an exponent: leave
        // it for whatever comes next to make sense of, or not.
        self.at = before;
        Some((1, 1))
    }

    /// A run of digits as a number, or `None` without consuming any.
    pub(crate) fn unsigned(&mut self) -> Option<i32> {
        let start = self.at;
        while self.chars.get(self.at).is_some_and(char::is_ascii_digit) {
            self.at += 1;
        }
        if self.at == start {
            return None;
        }
        self.chars[start..self.at]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }
}

/// Dimensions of one unit symbol, with an SI prefix allowed in front.
fn symbol_dimensions(symbol: &str) -> Option<Dim> {
    if let Some(dim) = bare_symbol(symbol) {
        return Some(dim);
    }
    // A whole-symbol match wins over a prefix reading, so `cd` is the
    // candela and `min` the minute; `mm`, `kPa` and `MOhm` land here.
    // Every SI prefix is here, the two-letter `da` first so it is tried
    // before `d`, and the reading is the first prefix that leaves a
    // symbol behind - scale factors do not matter, only that the letter
    // is a prefix at all.
    let prefixes: [&str; 25] = [
        "da", "Q", "R", "Y", "Z", "E", "P", "T", "G", "M", "k", "h", "d", "c", "m", "u", "µ", "n",
        "p", "f", "a", "z", "y", "r", "q",
    ];
    prefixes
        .iter()
        .filter(|prefix| symbol.starts_with(**prefix))
        .find_map(|prefix| bare_symbol(&symbol[prefix.len()..]))
}

/// The dimensions of an unprefixed symbol: SI base and derived units,
/// with angles dimensionless and scale factors ignored.
fn bare_symbol(symbol: &str) -> Option<Dim> {
    let dim = |values: [i32; BASES]| Some(Dim::of(values));
    match symbol {
        "1" | "rad" | "sr" | "deg" | "%" => dim([0, 0, 0, 0, 0, 0, 0]),
        "m" => dim([1, 0, 0, 0, 0, 0, 0]),
        "g" | "kg" => dim([0, 1, 0, 0, 0, 0, 0]),
        "s" | "min" | "h" | "d" => dim([0, 0, 1, 0, 0, 0, 0]),
        "A" => dim([0, 0, 0, 1, 0, 0, 0]),
        "K" | "degC" => dim([0, 0, 0, 0, 1, 0, 0]),
        "mol" => dim([0, 0, 0, 0, 0, 1, 0]),
        "cd" => dim([0, 0, 0, 0, 0, 0, 1]),
        "Hz" | "Bq" => dim([0, 0, -1, 0, 0, 0, 0]),
        "N" => dim([1, 1, -2, 0, 0, 0, 0]),
        "Pa" | "bar" => dim([-1, 1, -2, 0, 0, 0, 0]),
        "J" | "eV" => dim([2, 1, -2, 0, 0, 0, 0]),
        "W" => dim([2, 1, -3, 0, 0, 0, 0]),
        "C" => dim([0, 0, 1, 1, 0, 0, 0]),
        "V" => dim([2, 1, -3, -1, 0, 0, 0]),
        "F" => dim([-2, -1, 4, 2, 0, 0, 0]),
        "Ohm" | "ohm" => dim([2, 1, -3, -2, 0, 0, 0]),
        "S" => dim([-2, -1, 3, 2, 0, 0, 0]),
        "Wb" => dim([2, 1, -2, -1, 0, 0, 0]),
        "T" => dim([0, 1, -2, -1, 0, 0, 0]),
        "H" => dim([2, 1, -2, -2, 0, 0, 0]),
        "lm" => dim([0, 0, 0, 0, 0, 0, 1]),
        "lx" => dim([-2, 0, 0, 0, 0, 0, 1]),
        "Gy" | "Sv" => dim([2, 0, -2, 0, 0, 0, 0]),
        "kat" => dim([0, 0, -1, 0, 0, 1, 0]),
        "L" | "l" => dim([3, 0, 0, 0, 0, 0, 0]),
        _ => None,
    }
}
