//! A small math-expression language.
//!
//! One parser serves three jobs:
//! - composite-model formulas, where identifiers are component prefixes (`(g1 + l1) * e1`)
//! - lmfit-style parameter constraints, where identifiers are parameters (`2 * g1_sigma`,
//!   or `D1.g1_sigma` to reach into another dataset during a global fit)
//! - expression models, where identifiers are `x` plus the model's parameters
//!
//! Parsing produces an [`Expr`] with named variables; [`Expr::compile`] resolves the
//! names to slot indices once so repeated evaluation is a cheap tree walk.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Exp,
    Ln,
    Log10,
    Sqrt,
    Abs,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Erf,
    Erfc,
    Sign,
    Pow,
    Min,
    Max,
    Atan2,
}

impl Func {
    fn from_name(name: &str) -> Option<(Func, usize)> {
        Some(match name {
            "exp" => (Func::Exp, 1),
            "ln" | "log" => (Func::Ln, 1),
            "log10" => (Func::Log10, 1),
            "sqrt" => (Func::Sqrt, 1),
            "abs" => (Func::Abs, 1),
            "sin" => (Func::Sin, 1),
            "cos" => (Func::Cos, 1),
            "tan" => (Func::Tan, 1),
            "asin" => (Func::Asin, 1),
            "acos" => (Func::Acos, 1),
            "atan" => (Func::Atan, 1),
            "sinh" => (Func::Sinh, 1),
            "cosh" => (Func::Cosh, 1),
            "tanh" => (Func::Tanh, 1),
            "erf" => (Func::Erf, 1),
            "erfc" => (Func::Erfc, 1),
            "sign" => (Func::Sign, 1),
            "pow" => (Func::Pow, 2),
            "min" => (Func::Min, 2),
            "max" => (Func::Max, 2),
            "atan2" => (Func::Atan2, 2),
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Func::Exp => "exp",
            Func::Ln => "ln",
            Func::Log10 => "log10",
            Func::Sqrt => "sqrt",
            Func::Abs => "abs",
            Func::Sin => "sin",
            Func::Cos => "cos",
            Func::Tan => "tan",
            Func::Asin => "asin",
            Func::Acos => "acos",
            Func::Atan => "atan",
            Func::Sinh => "sinh",
            Func::Cosh => "cosh",
            Func::Tanh => "tanh",
            Func::Erf => "erf",
            Func::Erfc => "erfc",
            Func::Sign => "sign",
            Func::Pow => "pow",
            Func::Min => "min",
            Func::Max => "max",
            Func::Atan2 => "atan2",
        }
    }

    fn apply(self, a: &[f64]) -> f64 {
        match self {
            Func::Exp => a[0].exp(),
            Func::Ln => a[0].ln(),
            Func::Log10 => a[0].log10(),
            Func::Sqrt => a[0].sqrt(),
            Func::Abs => a[0].abs(),
            Func::Sin => a[0].sin(),
            Func::Cos => a[0].cos(),
            Func::Tan => a[0].tan(),
            Func::Asin => a[0].asin(),
            Func::Acos => a[0].acos(),
            Func::Atan => a[0].atan(),
            Func::Sinh => a[0].sinh(),
            Func::Cosh => a[0].cosh(),
            Func::Tanh => a[0].tanh(),
            Func::Erf => erf(a[0]),
            Func::Erfc => 1.0 - erf(a[0]),
            Func::Sign => {
                if a[0] > 0.0 {
                    1.0
                } else if a[0] < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }
            Func::Pow => a[0].powf(a[1]),
            Func::Min => a[0].min(a[1]),
            Func::Max => a[0].max(a[1]),
            Func::Atan2 => a[0].atan2(a[1]),
        }
    }
}

/// Error function, Abramowitz & Stegun 7.1.26 refined with a Taylor series near 0.
/// Accurate to ~1e-7, plenty for model shapes.
pub fn erf(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    let ax = x.abs();
    let v = if ax < 0.5 {
        // Series: 2/sqrt(pi) * sum (-1)^n x^(2n+1) / (n! (2n+1))
        let x2 = ax * ax;
        let mut term = ax;
        let mut sum = ax;
        for n in 1..20 {
            term *= -x2 / n as f64;
            sum += term / (2 * n + 1) as f64;
        }
        sum * std::f64::consts::FRAC_2_SQRT_PI
    } else {
        let t = 1.0 / (1.0 + 0.3275911 * ax);
        let poly = t
            * (0.254829592
                + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
        1.0 - poly * (-ax * ax).exp()
    };
    if x < 0.0 { -v } else { v }
}

/// Parsed expression with variables still referenced by name.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f64),
    Var(String),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Expr>),
}

/// An expression whose variables are resolved to indices into a value slice.
#[derive(Clone, Debug)]
pub enum Compiled {
    Num(f64),
    Var(usize),
    Neg(Box<Compiled>),
    Bin(BinOp, Box<Compiled>, Box<Compiled>),
    Call(Func, Vec<Compiled>),
}

impl Compiled {
    pub fn eval(&self, vars: &[f64]) -> f64 {
        match self {
            Compiled::Num(v) => *v,
            Compiled::Var(i) => vars[*i],
            Compiled::Neg(e) => -e.eval(vars),
            Compiled::Bin(op, a, b) => {
                let (a, b) = (a.eval(vars), b.eval(vars));
                match op {
                    BinOp::Add => a + b,
                    BinOp::Sub => a - b,
                    BinOp::Mul => a * b,
                    BinOp::Div => a / b,
                    BinOp::Pow => a.powf(b),
                }
            }
            Compiled::Call(f, args) => {
                let mut buf = [0.0; 2];
                for (slot, a) in buf.iter_mut().zip(args) {
                    *slot = a.eval(vars);
                }
                f.apply(&buf[..args.len()])
            }
        }
    }
}

impl Expr {
    pub fn parse(src: &str) -> Result<Expr, String> {
        let tokens = tokenize(src)?;
        let mut p = Parser { tokens, pos: 0 };
        let e = p.expr()?;
        if p.pos != p.tokens.len() {
            return Err(format!("unexpected `{}`", p.tokens[p.pos]));
        }
        Ok(e)
    }

    /// Distinct variable names in order of first appearance.
    pub fn vars(&self) -> Vec<String> {
        fn walk(e: &Expr, out: &mut Vec<String>) {
            match e {
                Expr::Num(_) => {}
                Expr::Var(v) => {
                    if !out.contains(v) {
                        out.push(v.clone());
                    }
                }
                Expr::Neg(a) => walk(a, out),
                Expr::Bin(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                Expr::Call(_, args) => args.iter().for_each(|a| walk(a, out)),
            }
        }
        let mut out = Vec::new();
        walk(self, &mut out);
        out
    }

    pub fn compile(&self, resolve: &dyn Fn(&str) -> Option<usize>) -> Result<Compiled, String> {
        Ok(match self {
            Expr::Num(v) => Compiled::Num(*v),
            Expr::Var(name) => match resolve(name) {
                Some(i) => Compiled::Var(i),
                None => return Err(format!("unknown name `{name}`")),
            },
            Expr::Neg(a) => Compiled::Neg(Box::new(a.compile(resolve)?)),
            Expr::Bin(op, a, b) => Compiled::Bin(
                *op,
                Box::new(a.compile(resolve)?),
                Box::new(b.compile(resolve)?),
            ),
            Expr::Call(f, args) => Compiled::Call(
                *f,
                args.iter()
                    .map(|a| a.compile(resolve))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Num(v) => write!(f, "{v}"),
            Expr::Var(v) => write!(f, "{v}"),
            Expr::Neg(a) => write!(f, "-({a})"),
            Expr::Bin(op, a, b) => {
                let s = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Pow => "^",
                };
                write!(f, "({a} {s} {b})")
            }
            Expr::Call(func, args) => {
                write!(f, "{}(", func.name())?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{a}")?;
                }
                write!(f, ")")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::Num(v) => write!(f, "{v}"),
            Tok::Ident(s) => write!(f, "{s}"),
            Tok::Op(c) => write!(f, "{c}"),
        }
    }
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // Exponent: 1e-3, 2.5E+4
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let s: String = chars[start..i].iter().collect();
            let v = s.parse::<f64>().map_err(|_| format!("bad number `{s}`"))?;
            out.push(Tok::Num(v));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            // Dots are allowed inside names so `D1.g1_sigma` is one identifier.
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else if c == '*' && chars.get(i + 1) == Some(&'*') {
            out.push(Tok::Op('^'));
            i += 2;
        } else if "+-*/^(),".contains(c) {
            out.push(Tok::Op(c));
            i += 1;
        } else {
            return Err(format!("unexpected character `{c}`"));
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek_op(&self) -> Option<char> {
        match self.tokens.get(self.pos) {
            Some(Tok::Op(c)) => Some(*c),
            _ => None,
        }
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.peek_op() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected `{c}`"))
        }
    }

    // expr := term (('+'|'-') term)*
    fn expr(&mut self) -> Result<Expr, String> {
        let mut lhs = self.term()?;
        while let Some(c @ ('+' | '-')) = self.peek_op() {
            self.pos += 1;
            let rhs = self.term()?;
            let op = if c == '+' { BinOp::Add } else { BinOp::Sub };
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    // term := unary (('*'|'/') unary)*
    fn term(&mut self) -> Result<Expr, String> {
        let mut lhs = self.unary()?;
        while let Some(c @ ('*' | '/')) = self.peek_op() {
            self.pos += 1;
            let rhs = self.unary()?;
            let op = if c == '*' { BinOp::Mul } else { BinOp::Div };
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    // unary := ('-'|'+') unary | power
    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek_op() {
            Some('-') => {
                self.pos += 1;
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            Some('+') => {
                self.pos += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    // power := atom ('^' unary)?   (right associative, binds tighter than unary minus on the left)
    fn power(&mut self) -> Result<Expr, String> {
        let base = self.atom()?;
        if self.peek_op() == Some('^') {
            self.pos += 1;
            let exp = self.unary()?;
            return Ok(Expr::Bin(BinOp::Pow, Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<Expr, String> {
        let Some(tok) = self.tokens.get(self.pos).cloned() else {
            return Err("unexpected end of expression".into());
        };
        self.pos += 1;
        match tok {
            Tok::Num(v) => Ok(Expr::Num(v)),
            Tok::Ident(name) => {
                if self.peek_op() == Some('(') {
                    self.pos += 1;
                    let Some((func, arity)) = Func::from_name(&name) else {
                        return Err(format!("unknown function `{name}`"));
                    };
                    let mut args = Vec::new();
                    if self.peek_op() != Some(')') {
                        args.push(self.expr()?);
                        while self.peek_op() == Some(',') {
                            self.pos += 1;
                            args.push(self.expr()?);
                        }
                    }
                    self.expect(')')?;
                    if args.len() != arity {
                        return Err(format!(
                            "`{name}` takes {arity} argument(s), got {}",
                            args.len()
                        ));
                    }
                    Ok(Expr::Call(func, args))
                } else {
                    Ok(match name.as_str() {
                        "pi" => Expr::Num(std::f64::consts::PI),
                        "inf" => Expr::Num(f64::INFINITY),
                        _ => Expr::Var(name),
                    })
                }
            }
            Tok::Op('(') => {
                let e = self.expr()?;
                self.expect(')')?;
                Ok(e)
            }
            Tok::Op(c) => Err(format!("unexpected `{c}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str, names: &[&str], vals: &[f64]) -> f64 {
        let e = Expr::parse(src).unwrap();
        let c = e.compile(&|n| names.iter().position(|m| *m == n)).unwrap();
        c.eval(vals)
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(eval("1 + 2 * 3", &[], &[]), 7.0);
        assert_eq!(eval("(1 + 2) * 3", &[], &[]), 9.0);
        assert_eq!(eval("2 ^ 3 ^ 2", &[], &[]), 512.0);
        assert_eq!(eval("-2 ^ 2", &[], &[]), -4.0);
        assert_eq!(eval("2 ** 3", &[], &[]), 8.0);
        assert_eq!(eval("8 / 4 / 2", &[], &[]), 1.0);
        assert_eq!(eval("1e3 + 2.5E-1", &[], &[]), 1000.25);
    }

    #[test]
    fn variables_functions_and_dotted_names() {
        let v = eval(
            "a * exp(-x / tau) + D1.c",
            &["x", "a", "tau", "D1.c"],
            &[1.0, 2.0, 1.0, 0.5],
        );
        assert!((v - (2.0 * (-1.0f64).exp() + 0.5)).abs() < 1e-12);
        assert_eq!(eval("max(a, 3)", &["a"], &[5.0]), 5.0);
        assert!((eval("sin(pi / 2)", &[], &[]) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn vars_lists_distinct_names_in_order() {
        let e = Expr::parse("b * x + a * x + b").unwrap();
        assert_eq!(e.vars(), vec!["b", "x", "a"]);
    }

    #[test]
    fn errors_are_reported() {
        assert!(Expr::parse("1 +").is_err());
        assert!(Expr::parse("foo(1)").is_err());
        assert!(Expr::parse("pow(1)").is_err());
        assert!(Expr::parse("(1").is_err());
        assert!(Expr::parse("1 $ 2").is_err());
        assert!(Expr::parse("a").unwrap().compile(&|_| None).is_err());
    }

    #[test]
    fn erf_matches_reference_values() {
        for (x, want) in [
            (0.0, 0.0),
            (0.3, 0.328626759),
            (1.0, 0.842700793),
            (-2.0, -0.995322265),
        ] {
            assert!((erf(x) - want).abs() < 2e-7, "erf({x})");
        }
    }
}
