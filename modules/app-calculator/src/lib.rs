//! C4 Calculator: recursive-descent expression evaluator.
//!
//! Actions
//! * `launch` – usage help
//! * `eval`   – payload is an expression, e.g. `2*(3+4)^2 - sqrt(16)`
//! * `save`   – evaluate, then ask Notes (via the ACL-checked IPC bus) to
//!              append `expr = result`

use mandala_module_api::{declare_module, Host, Module, ModuleContext, ModuleReply};

const MAX_INPUT: usize = 512;
const MAX_DEPTH: usize = 64;

pub struct Calculator {
    host: Host,
}

impl Calculator {
    pub fn new() -> Self {
        Self { host: Host::detached("app-calculator") }
    }
}

impl Default for Calculator {
    fn default() -> Self {
        Self::new()
    }
}

const HELP: &str = "CALCULATOR\n\n= 2*(3+4)\n= sqrt(2)^2\n= sin(pi/2) + ln(e)\n= 10 % 3\n= save 19.99*3   (also logs to Notes)\n\nOperators: + - * / % ^   Functions: sqrt sin cos tan ln log exp abs floor ceil round   Constants: pi e";

impl Module for Calculator {
    fn id(&self) -> &'static str {
        "app-calculator"
    }
    fn name(&self) -> &'static str {
        "Calculator"
    }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> {
        self.host = ctx.host;
        Ok(())
    }
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        match action {
            "launch" => ModuleReply::ok(HELP),
            "eval" => match evaluate(payload) {
                Ok(v) => ModuleReply::ok(format!("{} = {}", payload.trim(), format_number(v))),
                Err(e) => ModuleReply::err(e),
            },
            "save" => match evaluate(payload) {
                Ok(v) => {
                    let line = format!("{} = {}", payload.trim(), format_number(v));
                    match self.host.send("app-notes", "append", &line) {
                        Ok(()) => ModuleReply::ok(format!("{line}\n(sent to Notes)")),
                        Err(code) => ModuleReply::ok(format!("{line}\n(could not reach Notes: code {code})")),
                    }
                }
                Err(e) => ModuleReply::err(e),
            },
            other => ModuleReply::err(format!("unknown action `{other}`")),
        }
    }
}

declare_module!(Calculator::new);

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

pub fn evaluate(input: &str) -> Result<f64, String> {
    if input.len() > MAX_INPUT {
        return Err(format!("expression too long (max {MAX_INPUT} chars)"));
    }
    let mut p = Parser { chars: input.chars().collect(), pos: 0, depth: 0 };
    p.skip_ws();
    if p.at_end() {
        return Err("empty expression".into());
    }
    let v = p.expr()?;
    p.skip_ws();
    if !p.at_end() {
        return Err(format!("unexpected `{}` at position {}", p.chars[p.pos], p.pos + 1));
    }
    check(v)
}

fn check(v: f64) -> Result<f64, String> {
    if v.is_nan() {
        Err("undefined result".into())
    } else if v.is_infinite() {
        Err("result is too large".into())
    } else {
        Ok(v)
    }
}

pub fn format_number(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    if a >= 1e15 || a < 1e-9 {
        return format!("{v:e}");
    }
    let s = format!("{v:.10}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn at_end(&self) -> bool {
        self.pos >= self.chars.len()
    }
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
    fn skip_ws(&mut self) {
        while self.peek().map_or(false, char::is_whitespace) {
            self.pos += 1;
        }
    }
    fn eat(&mut self, c: char) -> bool {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Err("expression nested too deeply".into())
        } else {
            Ok(())
        }
    }

    // expr = term (('+' | '-') term)*
    fn expr(&mut self) -> Result<f64, String> {
        self.enter()?;
        let mut v = self.term()?;
        loop {
            if self.eat('+') {
                v += self.term()?;
            } else if self.eat('-') {
                v -= self.term()?;
            } else {
                break;
            }
        }
        self.depth -= 1;
        check(v)
    }

    // term = unary (('*' | '/' | '%') unary)*
    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.unary()?;
        loop {
            if self.eat('*') {
                v *= self.unary()?;
            } else if self.eat('/') {
                let d = self.unary()?;
                if d == 0.0 {
                    return Err("division by zero".into());
                }
                v /= d;
            } else if self.eat('%') {
                let d = self.unary()?;
                if d == 0.0 {
                    return Err("modulo by zero".into());
                }
                v %= d;
            } else {
                break;
            }
            v = check(v)?;
        }
        Ok(v)
    }

    // unary = ('-' | '+') unary | power      (so -2^2 == -4)
    fn unary(&mut self) -> Result<f64, String> {
        self.enter()?;
        let r = if self.eat('-') {
            self.unary().map(|v| -v)
        } else if self.eat('+') {
            self.unary()
        } else {
            self.power()
        };
        self.depth -= 1;
        r
    }

    // power = atom ('^' unary)?              (right-associative)
    fn power(&mut self) -> Result<f64, String> {
        let base = self.atom()?;
        if self.eat('^') {
            let exp = self.unary()?;
            return check(base.powf(exp));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<f64, String> {
        self.skip_ws();
        match self.peek() {
            None => Err("unexpected end of expression".into()),
            Some('(') => {
                self.pos += 1;
                let v = self.expr()?;
                if !self.eat(')') {
                    return Err("missing `)`".into());
                }
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() || c == '.' => self.number(),
            Some(c) if c.is_ascii_alphabetic() => self.ident(),
            Some(c) => Err(format!("unexpected `{c}` at position {}", self.pos + 1)),
        }
    }

    fn number(&mut self) -> Result<f64, String> {
        let start = self.pos;
        while self.peek().map_or(false, |c| c.is_ascii_digit() || c == '.') {
            self.pos += 1;
        }
        // optional exponent: e/E followed by optional sign and digits
        if matches!(self.peek(), Some('e') | Some('E')) {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.pos += 1;
            }
            if self.peek().map_or(false, |c| c.is_ascii_digit()) {
                while self.peek().map_or(false, |c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
            } else {
                self.pos = save; // it was the constant `e`, not an exponent
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse::<f64>().map_err(|_| format!("invalid number `{text}`"))
    }

    fn ident(&mut self) -> Result<f64, String> {
        let start = self.pos;
        while self.peek().map_or(false, |c| c.is_ascii_alphanumeric()) {
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().collect::<String>().to_ascii_lowercase();
        self.skip_ws();
        if self.peek() == Some('(') {
            self.pos += 1;
            let arg = self.expr()?;
            if !self.eat(')') {
                return Err("missing `)`".into());
            }
            return apply(&name, arg);
        }
        match name.as_str() {
            "pi" => Ok(std::f64::consts::PI),
            "e" => Ok(std::f64::consts::E),
            _ => Err(format!("unknown name `{name}`")),
        }
    }
}

fn apply(name: &str, x: f64) -> Result<f64, String> {
    let v = match name {
        "sqrt" if x < 0.0 => return Err("sqrt of a negative number".into()),
        "sqrt" => x.sqrt(),
        "sin" => x.sin(),
        "cos" => x.cos(),
        "tan" => x.tan(),
        "ln" if x <= 0.0 => return Err("ln of a non-positive number".into()),
        "ln" => x.ln(),
        "log" if x <= 0.0 => return Err("log of a non-positive number".into()),
        "log" => x.log10(),
        "exp" => x.exp(),
        "abs" => x.abs(),
        "floor" => x.floor(),
        "ceil" => x.ceil(),
        "round" => x.round(),
        _ => return Err(format!("unknown function `{name}`")),
    };
    check(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> f64 {
        evaluate(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(ev("1+2*3"), 7.0);
        assert_eq!(ev("(1+2)*3"), 9.0);
        assert_eq!(ev("10/4"), 2.5);
        assert_eq!(ev("10 % 3"), 1.0);
        assert_eq!(ev("2^3^2"), 512.0); // right-assoc
        assert_eq!(ev("-2^2"), -4.0);
        assert_eq!(ev("2*-3"), -6.0);
        assert_eq!(ev("--4"), 4.0);
        assert_eq!(ev("  7 -  2 "), 5.0);
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(ev("sqrt(16)"), 4.0);
        assert!((ev("sin(pi/2)") - 1.0).abs() < 1e-12);
        assert!((ev("ln(e)") - 1.0).abs() < 1e-12);
        assert!((ev("log(1000)") - 3.0).abs() < 1e-12);
        assert_eq!(ev("abs(-3)+floor(2.9)+ceil(2.1)+round(2.5)"), 3.0 + 2.0 + 3.0 + 3.0);
        assert_eq!(ev("SQRT(9)"), 3.0);
    }

    #[test]
    fn scientific_notation_vs_constant_e() {
        assert_eq!(ev("1e3"), 1000.0);
        assert_eq!(ev("2.5E-1"), 0.25);
        assert!((ev("2*e") - 2.0 * std::f64::consts::E).abs() < 1e-12);
    }

    #[test]
    fn errors_are_reported_not_panics() {
        for bad in ["", "   ", "1/0", "5 % 0", "sqrt(-1)", "ln(0)", "1+", "(1", "1)", "foo", "foo(1)", "2 $ 3", "1..2", "((((", "10^1000"] {
            assert!(evaluate(bad).is_err(), "{bad:?} should fail");
        }
    }

    #[test]
    fn depth_and_length_limits() {
        let deep = format!("{}1{}", "(".repeat(MAX_DEPTH + 5), ")".repeat(MAX_DEPTH + 5));
        assert!(evaluate(&deep).unwrap_err().contains("deeply"));
        assert!(evaluate(&"1+".repeat(400)).is_err());
        let minus = "-".repeat(10_000);
        assert!(evaluate(&minus).is_err());
    }

    #[test]
    fn number_formatting() {
        assert_eq!(format_number(4.0), "4");
        assert_eq!(format_number(2.5), "2.5");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(-0.0), "0");
        assert_eq!(format_number(1e20), "1e20");
        assert_eq!(format_number(-7.25), "-7.25");
    }

    #[test]
    fn module_actions() {
        let mut c = Calculator::new();
        assert!(c.handle("launch", "").text.contains("CALCULATOR"));
        assert_eq!(c.handle("eval", "2+2").text, "2+2 = 4");
        assert!(!c.handle("eval", "2+").ok);
        // Detached host => note could not be sent, but result is still shown.
        let r = c.handle("save", "3*3");
        assert!(r.ok && r.text.contains("3*3 = 9") && r.text.contains("could not reach Notes"));
        assert!(!c.handle("nope", "").ok);
    }
}
