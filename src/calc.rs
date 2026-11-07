//! Native math evaluator. No external process, no allocation beyond the token list.
//!
//! Grammar (lowest to highest precedence):
//!   expr    = term (("+" | "-") term)*
//!   term    = unary (("*" | "/" | "%") unary | implicit)*   implicit: `2pi`, `3(4+1)`
//!   unary   = ("-" | "+") unary | power
//!   power   = postfix ("^" unary)?                           right-associative
//!   postfix = primary "!"*
//!   primary = number | const | func "(" args ")" | "(" expr ")"

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            // radix literals: 0x 0b 0o
            if ch == '0' && i + 1 < c.len() && matches!(c[i + 1], 'x' | 'X' | 'b' | 'B' | 'o' | 'O') {
                let radix = match c[i + 1].to_ascii_lowercase() {
                    'x' => 16,
                    'b' => 2,
                    _ => 8,
                };
                let start = i + 2;
                let mut j = start;
                while j < c.len() && (c[j].is_digit(radix) || c[j] == '_') {
                    j += 1;
                }
                let digits: String = c[start..j].iter().filter(|&&d| d != '_').collect();
                if digits.is_empty() {
                    return Err("empty radix literal".into());
                }
                let v = u64::from_str_radix(&digits, radix).map_err(|e| e.to_string())?;
                out.push(Tok::Num(v as f64));
                i = j;
                continue;
            }
            let start = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.' || c[i] == '_') {
                i += 1;
            }
            // exponent: 1e5, 2.5e-3 (only when followed by a digit, so `2e` stays 2*e)
            if i < c.len() && matches!(c[i], 'e' | 'E') {
                let mut j = i + 1;
                if j < c.len() && matches!(c[j], '+' | '-') {
                    j += 1;
                }
                if j < c.len() && c[j].is_ascii_digit() {
                    i = j;
                    while i < c.len() && c[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = c[start..i].iter().filter(|&&d| d != '_').collect();
            out.push(Tok::Num(text.parse().map_err(|_| format!("bad number `{text}`"))?));
        } else if ch.is_alphabetic() {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(c[start..i].iter().collect::<String>().to_lowercase()));
        } else {
            let op = match ch {
                '+' | '-' | '/' | '%' | '^' | '(' | ')' | ',' | '!' => ch,
                '*' if c.get(i + 1) == Some(&'*') => {
                    i += 1;
                    '^'
                }
                '*' | '×' | '·' => '*',
                '÷' => '/',
                '−' => '-',
                _ => return Err(format!("unexpected `{ch}`")),
            };
            out.push(Tok::Op(op));
            i += 1;
        }
    }
    Ok(out)
}

struct Parser {
    t: Vec<Tok>,
    p: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.p)
    }
    fn eat(&mut self, op: char) -> bool {
        if self.peek() == Some(&Tok::Op(op)) {
            self.p += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        loop {
            if self.eat('+') {
                v += self.term()?;
            } else if self.eat('-') {
                v -= self.term()?;
            } else {
                return Ok(v);
            }
        }
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.unary()?;
        loop {
            if self.eat('*') {
                v *= self.unary()?;
            } else if self.eat('/') {
                v /= self.unary()?;
            } else if self.eat('%') {
                v %= self.unary()?;
            } else if matches!(self.peek(), Some(Tok::Ident(_)) | Some(Tok::Op('('))) {
                v *= self.power()?; // implicit multiplication
            } else {
                return Ok(v);
            }
        }
    }

    fn unary(&mut self) -> Result<f64, String> {
        if self.eat('-') {
            return Ok(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.postfix()?;
        if self.eat('^') {
            return Ok(base.powf(self.unary()?));
        }
        Ok(base)
    }

    fn postfix(&mut self) -> Result<f64, String> {
        let mut v = self.primary()?;
        while self.eat('!') {
            if v < 0.0 || v.fract() != 0.0 || v > 170.0 {
                return Err("factorial needs an integer 0..=170".into());
            }
            v = (1..=v as u64).map(|n| n as f64).product();
        }
        Ok(v)
    }

    fn primary(&mut self) -> Result<f64, String> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.p += 1;
                Ok(n)
            }
            Some(Tok::Op('(')) => {
                self.p += 1;
                let v = self.expr()?;
                if !self.eat(')') {
                    return Err("missing `)`".into());
                }
                Ok(v)
            }
            Some(Tok::Ident(name)) => {
                self.p += 1;
                if self.eat('(') {
                    let mut args = Vec::new();
                    if !self.eat(')') {
                        loop {
                            args.push(self.expr()?);
                            if self.eat(')') {
                                break;
                            }
                            if !self.eat(',') {
                                return Err("expected `,` or `)`".into());
                            }
                        }
                    }
                    call(&name, &args)
                } else {
                    constant(&name).ok_or_else(|| format!("unknown name `{name}`"))
                }
            }
            Some(Tok::Op(o)) => Err(format!("unexpected `{o}`")),
            None => Err("unexpected end".into()),
        }
    }
}

fn constant(name: &str) -> Option<f64> {
    use std::f64::consts::*;
    Some(match name {
        "pi" | "π" => PI,
        "e" => E,
        "tau" | "τ" => TAU,
        "phi" => 1.618_033_988_749_895,
        "inf" => f64::INFINITY,
        _ => return None,
    })
}

fn call(name: &str, a: &[f64]) -> Result<f64, String> {
    let one = |f: fn(f64) -> f64| -> Result<f64, String> {
        match a {
            [x] => Ok(f(*x)),
            _ => Err(format!("{name}() takes 1 argument")),
        }
    };
    match name {
        "sqrt" => one(f64::sqrt),
        "cbrt" => one(f64::cbrt),
        "abs" => one(f64::abs),
        "sin" => one(f64::sin),
        "cos" => one(f64::cos),
        "tan" => one(f64::tan),
        "asin" => one(f64::asin),
        "acos" => one(f64::acos),
        "atan" => one(f64::atan),
        "sinh" => one(f64::sinh),
        "cosh" => one(f64::cosh),
        "tanh" => one(f64::tanh),
        "ln" => one(f64::ln),
        "log" | "log10" => one(f64::log10),
        "log2" => one(f64::log2),
        "exp" => one(f64::exp),
        "floor" => one(f64::floor),
        "ceil" => one(f64::ceil),
        "round" => one(f64::round),
        "trunc" => one(f64::trunc),
        "deg" => one(f64::to_degrees),
        "rad" => one(f64::to_radians),
        "min" if !a.is_empty() => Ok(a.iter().cloned().fold(f64::INFINITY, f64::min)),
        "max" if !a.is_empty() => Ok(a.iter().cloned().fold(f64::NEG_INFINITY, f64::max)),
        "pow" if a.len() == 2 => Ok(a[0].powf(a[1])),
        "hypot" if a.len() == 2 => Ok(a[0].hypot(a[1])),
        _ => Err(format!("unknown function `{name}`")),
    }
}

pub fn eval(src: &str) -> Result<f64, String> {
    let src = src.trim().trim_start_matches('=');
    let mut p = Parser { t: tokenize(src)?, p: 0 };
    let v = p.expr()?;
    if p.p != p.t.len() {
        return Err("trailing input".into());
    }
    if v.is_nan() {
        return Err("not a number".into());
    }
    Ok(v)
}

/// Decide whether a launcher query should be treated as math. A query qualifies when it
/// starts with `=`, or it evaluates cleanly and contains an operator or function call
/// (so a bare `42` or `pi` keeps searching apps instead).
pub fn try_query(q: &str) -> Option<f64> {
    let q = q.trim();
    if q.is_empty() {
        return None;
    }
    let forced = q.starts_with('=');
    let toks = tokenize(q.trim_start_matches('=')).ok()?;
    let has_op = toks.iter().any(|t| matches!(t, Tok::Op(_)));
    let has_num = toks.iter().any(|t| matches!(t, Tok::Num(_)));
    if !forced && !(has_op && (has_num || toks.len() > 2)) {
        return None;
    }
    eval(q).ok()
}

/// Format with up to `precision` decimal places, trailing zeros removed.
pub fn format(v: f64, precision: usize) -> String {
    if v.is_infinite() {
        return if v > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let a = v.abs();
    if a != 0.0 && (a >= 1e15 || a < 1e-6) {
        let s = format!("{:.*e}", precision.min(15), v);
        let (m, e) = s.split_once('e').unwrap();
        let m = if m.contains('.') { m.trim_end_matches('0').trim_end_matches('.') } else { m };
        return format!("{m}e{e}");
    }
    let s = format!("{:.*}", precision, v);
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" { "0".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn f(s: &str) -> String {
        format(eval(s).unwrap(), 10)
    }

    #[test]
    fn arithmetic() {
        assert_eq!(f("1+2*3"), "7");
        assert_eq!(f("(1+2)*3"), "9");
        assert_eq!(f("2^3^2"), "512");
        assert_eq!(f("-2^2"), "-4");
        assert_eq!(f("2^-1"), "0.5");
        assert_eq!(f("10 % 4"), "2");
        assert_eq!(f("2**10"), "1024");
        assert_eq!(f("5!"), "120");
        assert_eq!(f("1_000 * 3"), "3000");
        assert_eq!(f("12 × 7 ÷ 4"), "21");
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(f("sqrt(16)+abs(-2)"), "6");
        assert_eq!(f("2pi"), "6.2831853072");
        assert_eq!(f("max(1, 9, 3)"), "9");
        assert_eq!(f("log(1000)"), "3");
        assert_eq!(f("3(4+1)"), "15");
        assert_eq!(f("0xff + 0b11"), "258");
        assert_eq!(f("1.5e3"), "1500");
    }

    #[test]
    fn query_detection() {
        assert!(try_query("1+1").is_some());
        assert!(try_query("=pi").is_some());
        assert!(try_query("sqrt(2)").is_some());
        assert!(try_query("42").is_none());
        assert!(try_query("pi").is_none());
        assert!(try_query("safari").is_none());
        assert!(try_query("1password").is_none());
        assert!(try_query("2+").is_none());
        assert!(try_query("0/0").is_none());
    }

    #[test]
    fn formatting() {
        assert_eq!(format(0.1 + 0.2, 10), "0.3");
        assert_eq!(format(1e20, 10), "1e20");
        assert_eq!(format(-0.0, 10), "0");
        assert_eq!(format(1.0 / 3.0, 4), "0.3333");
    }
}
