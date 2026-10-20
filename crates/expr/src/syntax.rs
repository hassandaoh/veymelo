//! The language's words and sentences: numbers, strings, names, operators; statements
//! (`var`, assignments, `if`, `for`, `function`, `return`) and expressions with the usual
//! precedence. A program's value is its last expression's (or what it returns).

use std::rc::Rc;

/// A mistake in how something is written, at a character offset.
#[derive(Clone, Debug, PartialEq)]
pub struct SyntaxError {
    pub at: usize,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f64),
    Str(Rc<str>),
    Name(Rc<str>),
    /// Operators and punctuation.
    Op(&'static str),
    End,
}

const OPS: [&str; 33] = [
    "===", "!==", "**", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "++", "--", "+", "-", "*", "/", "%", "<", ">", "!", "=", "?", ":", ",", ";", ".", "(", ")", "[", "]",
];
const BRACES: [&str; 2] = ["{", "}"];

/// The words of `src`, each with where it starts.
pub fn lex(src: &str) -> Result<Vec<(Tok, usize)>, SyntaxError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // comments
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        let start = i;
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                i += 1;
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    i += 1;
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = chars[start..i].iter().collect();
            let n = text.parse::<f64>().map_err(|_| SyntaxError { at: start, message: format!("\"{text}\" is not a number") })?;
            out.push((Tok::Num(n), start));
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            i += 1;
            let mut s = String::new();
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    s.push(match chars[i] {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => other,
                    });
                } else {
                    s.push(chars[i]);
                }
                i += 1;
            }
            if i >= chars.len() {
                return Err(SyntaxError { at: start, message: "a string is not closed".into() });
            }
            i += 1;
            out.push((Tok::Str(s.into()), start));
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '$' {
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            out.push((Tok::Name(chars[start..i].iter().collect::<String>().into()), start));
            continue;
        }
        let rest: String = chars[i..chars.len().min(i + 3)].iter().collect();
        match OPS.iter().chain(BRACES.iter()).find(|op| rest.starts_with(**op)) {
            Some(op) => {
                out.push((Tok::Op(op), start));
                i += op.chars().count();
            }
            None => return Err(SyntaxError { at: start, message: format!("\"{c}\" means nothing here") }),
        }
    }
    out.push((Tok::End, chars.len()));
    Ok(out)
}

/// An expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f64),
    Str(Rc<str>),
    Bool(bool),
    Name(Rc<str>),
    Array(Vec<Expr>),
    /// An operator on one value (`-`, `!`, `+`).
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    /// `a ? b : c`
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    /// `a.b`
    Member(Box<Expr>, Rc<str>),
    /// `a[b]`
    Index(Box<Expr>, Box<Expr>),
    /// A function written in place: its parameters and body.
    Function(Rc<Vec<Rc<str>>>, Rc<Vec<Stmt>>),
}

/// A statement.
#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// `var x = e` (also `let` and `const`).
    Var(Rc<str>, Option<Expr>),
    /// `target op= e`: `=`, `+=`, `-=`, `*=`, `/=`.
    Assign(Expr, &'static str, Expr),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    For(Box<Stmt>, Expr, Box<Stmt>, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    Function(Rc<str>, Rc<Vec<Rc<str>>>, Rc<Vec<Stmt>>),
    Return(Option<Expr>),
    Break,
    Continue,
    Expr(Expr),
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].0
    }

    fn at(&self) -> usize {
        self.toks[self.i].1
    }

    fn is(&self, op: &str) -> bool {
        matches!(self.peek(), Tok::Op(o) if *o == op)
    }

    fn is_name(&self, name: &str) -> bool {
        matches!(self.peek(), Tok::Name(n) if &**n == name)
    }

    fn eat(&mut self, op: &str) -> bool {
        if self.is(op) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn fail<T>(&self, message: impl Into<String>) -> Result<T, SyntaxError> {
        Err(SyntaxError { at: self.at(), message: message.into() })
    }

    fn expect(&mut self, op: &str) -> Result<(), SyntaxError> {
        if self.eat(op) {
            Ok(())
        } else {
            let found = match self.peek() {
                Tok::End => "the end".to_string(),
                Tok::Num(n) => n.to_string(),
                Tok::Str(s) => format!("\"{s}\""),
                Tok::Name(n) => n.to_string(),
                Tok::Op(o) => format!("\"{o}\""),
            };
            self.fail(format!("\"{op}\" is missing before {found}"))
        }
    }

    /// After an item of a list: whether `close` ends it, else the comma before the next.
    fn list_ends(&mut self, close: &str) -> Result<bool, SyntaxError> {
        if self.eat(close) {
            return Ok(true);
        }
        // at the end it is the closing bracket that is missing, not a comma
        if matches!(self.peek(), Tok::End) {
            return self.fail(format!("\"{close}\" is missing before the end"));
        }
        self.expect(",")?;
        Ok(false)
    }

    fn name(&mut self) -> Result<Rc<str>, SyntaxError> {
        match self.peek().clone() {
            Tok::Name(n) => {
                self.i += 1;
                Ok(n)
            }
            _ => self.fail("a name is wanted here"),
        }
    }

    fn statements_until(&mut self, close: Option<&str>) -> Result<Vec<Stmt>, SyntaxError> {
        let mut out = Vec::new();
        loop {
            while self.eat(";") {}
            match (self.peek(), close) {
                (Tok::End, None) => return Ok(out),
                (Tok::End, Some(c)) => return self.fail(format!("\"{c}\" is missing at the end")),
                (Tok::Op(o), Some(c)) if *o == c => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => out.push(self.statement()?),
            }
        }
    }

    fn block(&mut self) -> Result<Vec<Stmt>, SyntaxError> {
        if self.eat("{") { self.statements_until(Some("}")) } else { Ok(vec![self.statement()?]) }
    }

    fn params(&mut self) -> Result<Rc<Vec<Rc<str>>>, SyntaxError> {
        self.expect("(")?;
        let mut ps = Vec::new();
        if !self.eat(")") {
            loop {
                ps.push(self.name()?);
                if self.eat(")") {
                    break;
                }
                self.expect(",")?;
            }
        }
        Ok(Rc::new(ps))
    }

    fn statement(&mut self) -> Result<Stmt, SyntaxError> {
        if self.is_name("var") || self.is_name("let") || self.is_name("const") {
            self.i += 1;
            let n = self.name()?;
            let init = if self.eat("=") { Some(self.expr()?) } else { None };
            return Ok(Stmt::Var(n, init));
        }
        if self.is_name("if") {
            self.i += 1;
            self.expect("(")?;
            let c = self.expr()?;
            self.expect(")")?;
            let yes = self.block()?;
            while self.eat(";") {}
            let no = if self.is_name("else") {
                self.i += 1;
                self.block()?
            } else {
                Vec::new()
            };
            return Ok(Stmt::If(c, yes, no));
        }
        if self.is_name("for") {
            self.i += 1;
            self.expect("(")?;
            let init = if self.is(";") { Stmt::Expr(Expr::Bool(true)) } else { self.statement()? };
            self.expect(";")?;
            let cond = if self.is(";") { Expr::Bool(true) } else { self.expr()? };
            self.expect(";")?;
            let step = if self.is(")") { Stmt::Expr(Expr::Bool(true)) } else { self.statement()? };
            self.expect(")")?;
            let body = self.block()?;
            return Ok(Stmt::For(Box::new(init), cond, Box::new(step), body));
        }
        if self.is_name("while") {
            self.i += 1;
            self.expect("(")?;
            let c = self.expr()?;
            self.expect(")")?;
            return Ok(Stmt::While(c, self.block()?));
        }
        if self.is_name("function") && matches!(self.toks.get(self.i + 1), Some((Tok::Name(_), _))) {
            self.i += 1;
            let n = self.name()?;
            let ps = self.params()?;
            self.expect("{")?;
            let body = self.statements_until(Some("}"))?;
            return Ok(Stmt::Function(n, ps, Rc::new(body)));
        }
        if self.is_name("return") {
            self.i += 1;
            let e = if self.is(";") || self.is("}") || matches!(self.peek(), Tok::End) { None } else { Some(self.expr()?) };
            return Ok(Stmt::Return(e));
        }
        if self.is_name("break") {
            self.i += 1;
            return Ok(Stmt::Break);
        }
        if self.is_name("continue") {
            self.i += 1;
            return Ok(Stmt::Continue);
        }
        let e = self.expr()?;
        for op in ["=", "+=", "-=", "*=", "/="] {
            if self.eat(op) {
                if !matches!(e, Expr::Name(_) | Expr::Member(..) | Expr::Index(..)) {
                    return self.fail("only a name, a member or an index can be given a value");
                }
                let v = self.expr()?;
                return Ok(Stmt::Assign(e, op, v));
            }
        }
        for (op, as_) in [("++", "+="), ("--", "-=")] {
            if self.eat(op) {
                return Ok(Stmt::Assign(e, as_, Expr::Num(1.0)));
            }
        }
        Ok(Stmt::Expr(e))
    }

    fn expr(&mut self) -> Result<Expr, SyntaxError> {
        let c = self.binary(0)?;
        if self.eat("?") {
            let a = self.expr()?;
            self.expect(":")?;
            let b = self.expr()?;
            return Ok(Expr::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    /// Binary operators by precedence, lowest first.
    fn binary(&mut self, level: usize) -> Result<Expr, SyntaxError> {
        const LEVELS: [&[&str]; 6] = [&["||"], &["&&"], &["==", "!=", "===", "!=="], &["<", ">", "<=", ">="], &["+", "-"], &["*", "/", "%"]];
        if level == LEVELS.len() {
            return self.power();
        }
        let mut left = self.binary(level + 1)?;
        loop {
            let Some(op) = LEVELS[level].iter().find(|op| self.is(op)).copied() else { return Ok(left) };
            self.i += 1;
            let right = self.binary(level + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn power(&mut self) -> Result<Expr, SyntaxError> {
        let base = self.unary()?;
        if self.eat("**") {
            let exp = self.power()?;
            return Ok(Expr::Binary("**", Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn unary(&mut self) -> Result<Expr, SyntaxError> {
        for op in ["-", "!", "+"] {
            if self.eat(op) {
                return Ok(Expr::Unary(op, Box::new(self.unary()?)));
            }
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, SyntaxError> {
        let mut e = self.primary()?;
        loop {
            if self.eat("(") {
                let mut args = Vec::new();
                if !self.eat(")") {
                    loop {
                        args.push(self.expr()?);
                        if self.list_ends(")")? {
                            break;
                        }
                    }
                }
                e = Expr::Call(Box::new(e), args);
            } else if self.eat("[") {
                let k = self.expr()?;
                self.expect("]")?;
                e = Expr::Index(Box::new(e), Box::new(k));
            } else if self.eat(".") {
                e = Expr::Member(Box::new(e), self.name()?);
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, SyntaxError> {
        let tok = self.peek().clone();
        self.i += 1;
        Ok(match tok {
            Tok::Num(n) => Expr::Num(n),
            Tok::Str(s) => Expr::Str(s),
            Tok::Name(n) if &*n == "true" => Expr::Bool(true),
            Tok::Name(n) if &*n == "false" => Expr::Bool(false),
            Tok::Name(n) if &*n == "function" => {
                let ps = self.params()?;
                self.expect("{")?;
                Expr::Function(ps, Rc::new(self.statements_until(Some("}"))?))
            }
            Tok::Name(n) => Expr::Name(n),
            Tok::Op("[") => {
                let mut items = Vec::new();
                if !self.eat("]") {
                    loop {
                        items.push(self.expr()?);
                        if self.list_ends("]")? {
                            break;
                        }
                    }
                }
                Expr::Array(items)
            }
            Tok::Op("(") => {
                let e = self.expr()?;
                self.expect(")")?;
                e
            }
            Tok::End => {
                self.i -= 1;
                return self.fail("the expression ends too soon");
            }
            Tok::Op(o) => {
                self.i -= 1;
                return self.fail(format!("\"{o}\" cannot start a value"));
            }
        })
    }
}

/// A program: its statements.
pub fn parse(src: &str) -> Result<Vec<Stmt>, SyntaxError> {
    let mut p = Parser { toks: lex(src)?, i: 0 };
    p.statements_until(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_and_shapes() {
        let p = parse("1 + 2 * 3 ** 2 - -4").unwrap();
        let Stmt::Expr(Expr::Binary("-", left, right)) = &p[0] else { panic!("{p:?}") };
        assert!(matches!(**right, Expr::Unary("-", _)));
        assert!(matches!(**left, Expr::Binary("+", _, _)));
        let p = parse(r#"var a = [1, 2]; thisComp.layer("Null 1").transform.position[0] + a[1]"#).unwrap();
        assert_eq!(p.len(), 2);
        assert!(matches!(&p[1], Stmt::Expr(Expr::Binary("+", _, _))));
        let p = parse("if (time > 1) { x = 1 } else x = 2; for (var i = 0; i < 3; i++) { y += i } function f(a, b) { return a * b }").unwrap();
        assert_eq!(p.len(), 3);
        assert!(matches!(p[1], Stmt::For(..)) && matches!(p[2], Stmt::Function(..)));
    }

    #[test]
    fn mistakes_say_where() {
        let e = parse("wiggle(2, 30").unwrap_err();
        assert_eq!((e.at, e.message.as_str()), (12, "\")\" is missing before the end"));
        assert_eq!(parse("1 + ").unwrap_err().message, "the expression ends too soon");
        assert_eq!(parse("a # b").unwrap_err().message, "\"#\" means nothing here");
        assert_eq!(parse("\"open").unwrap_err().message, "a string is not closed");
        assert_eq!(parse("3 = 4").unwrap_err().message, "only a name, a member or an index can be given a value");
        assert!(parse("// a note\n/* and more */ 42").is_ok());
    }
}
