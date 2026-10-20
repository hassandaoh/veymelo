//! Running a program: values, scopes, operators (on numbers and on arrays, part by part), and
//! the objects of the project an expression can reach.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use veymelo_project::{Comp, Group, Layer, Marker, Node, Property};

use crate::syntax::{Expr, Stmt};

/// A value while running.
#[derive(Clone, Debug)]
pub enum Val<'a> {
    Undef,
    Num(f64),
    Bool(bool),
    Str(Rc<str>),
    Arr(Rc<Vec<Val<'a>>>),
    Obj(Obj<'a>),
    Fun(Rc<Fun<'a>>),
}

/// Something of the project.
#[derive(Clone, Debug)]
pub enum Obj<'a> {
    Comp(&'a Comp, u64),
    Layer(&'a Layer, &'a Comp, u64),
    Group(&'a Group, &'a Layer, &'a Comp, u64),
    Prop(&'a Property, &'a Layer, &'a Comp, u64),
    /// A property's key (its index from 0).
    Key(&'a Property, &'a Layer, &'a Comp, u64, usize),
    /// A composition's markers, or a layer's (their times are the layer's own).
    Markers(&'a [Marker], Option<&'a Layer>),
    /// One of them (its index from 0).
    Marker(&'a Marker, Option<&'a Layer>, usize),
    Math,
    /// A path's points and tangents (a mask's, a shape's), or one made by createPath.
    Path(Rc<veymelo_keyframe::ShapePath>),
    /// A box: left, top, width, height (what sourceRectAtTime gives).
    Rect([f64; 4]),
}

/// Something that can be called.
#[derive(Debug)]
pub enum Fun<'a> {
    User(Rc<Vec<Rc<str>>>, Rc<Vec<Stmt>>),
    /// A built-in, by name.
    Builtin(&'static str),
    /// A method of a value, by name.
    Method(Val<'a>, Rc<str>),
}

impl<'a> Val<'a> {
    pub fn kind(&self) -> &'static str {
        match self {
            Val::Undef => "nothing",
            Val::Num(_) => "a number",
            Val::Bool(_) => "true or false",
            Val::Str(_) => "a string",
            Val::Arr(_) => "an array",
            Val::Obj(Obj::Comp(..)) => "a composition",
            Val::Obj(Obj::Layer(..)) => "a layer",
            Val::Obj(Obj::Group(..)) => "a group",
            Val::Obj(Obj::Prop(..)) => "a property",
            Val::Obj(Obj::Key(..)) => "a key",
            Val::Obj(Obj::Markers(..)) => "markers",
            Val::Obj(Obj::Marker(..)) => "a marker",
            Val::Obj(Obj::Math) => "Math",
            Val::Obj(Obj::Path(_)) => "a path",
            Val::Obj(Obj::Rect(_)) => "a box",
            Val::Fun(_) => "a function",
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Val::Undef => false,
            Val::Num(n) => *n != 0.0 && !n.is_nan(),
            Val::Bool(b) => *b,
            Val::Str(s) => !s.is_empty(),
            _ => true,
        }
    }

    pub fn arr(items: Vec<Val<'a>>) -> Val<'a> {
        Val::Arr(Rc::new(items))
    }

    pub fn nums(v: &[f64]) -> Val<'a> {
        Val::arr(v.iter().map(|x| Val::Num(*x)).collect())
    }
}

/// A mistake while running.
pub type Fail = String;

/// What a statement did: went on, returned, broke out, or continued.
pub enum Flow<'a> {
    Next(Option<Val<'a>>),
    Return(Val<'a>),
    Break,
    Continue,
}

/// The project side of running: values of properties, built-in functions, members of objects.
pub trait Host<'a> {
    /// The current value of something that stands for a value (a property, a key).
    fn resolve(&self, v: Val<'a>) -> Result<Val<'a>, Fail>;
    fn member(&self, of: &Val<'a>, name: &str) -> Result<Val<'a>, Fail>;
    fn call_builtin(&self, run: &Run<'a, '_>, name: &str, args: Vec<Val<'a>>) -> Result<Val<'a>, Fail>;
    fn call_method(&self, run: &Run<'a, '_>, on: Val<'a>, name: &str, args: Vec<Val<'a>>) -> Result<Val<'a>, Fail>;
    /// A name nobody set: `time`, `value`, `thisLayer`…
    fn global(&self, name: &str) -> Option<Val<'a>>;
}

/// One run of a program.
pub struct Run<'a, 'h> {
    pub host: &'h dyn Host<'a>,
    scopes: RefCell<Vec<HashMap<Rc<str>, Val<'a>>>>,
    steps: RefCell<u64>,
}

/// How many steps a program may take (so a loop that never ends, ends).
const MAX_STEPS: u64 = 2_000_000;

impl<'a, 'h> Run<'a, 'h> {
    pub fn new(host: &'h dyn Host<'a>) -> Run<'a, 'h> {
        Run { host, scopes: RefCell::new(vec![HashMap::new()]), steps: RefCell::new(0) }
    }

    fn step(&self) -> Result<(), Fail> {
        let mut s = self.steps.borrow_mut();
        *s += 1;
        if *s > MAX_STEPS { Err("the expression takes too long (a loop that does not end?)".into()) } else { Ok(()) }
    }

    fn lookup(&self, name: &str) -> Result<Val<'a>, Fail> {
        for scope in self.scopes.borrow().iter().rev() {
            if let Some(v) = scope.get(name) {
                return Ok(v.clone());
            }
        }
        self.host.global(name).ok_or_else(|| format!("there is nothing called \"{name}\""))
    }

    fn set(&self, name: &Rc<str>, v: Val<'a>) {
        let mut scopes = self.scopes.borrow_mut();
        for scope in scopes.iter_mut().rev() {
            if let Some(slot) = scope.get_mut(name) {
                *slot = v;
                return;
            }
        }
        scopes.first_mut().expect("a scope").insert(name.clone(), v);
    }

    fn declare(&self, name: &Rc<str>, v: Val<'a>) {
        self.scopes.borrow_mut().last_mut().expect("a scope").insert(name.clone(), v);
    }

    /// The value of a program: what it returns, or its last expression's value.
    pub fn program(&self, stmts: &[Stmt]) -> Result<Val<'a>, Fail> {
        // functions can be called before they are written
        for s in stmts {
            if let Stmt::Function(n, ps, body) = s {
                self.declare(n, Val::Fun(Rc::new(Fun::User(ps.clone(), body.clone()))));
            }
        }
        match self.block(stmts)? {
            Flow::Return(v) => Ok(v),
            Flow::Next(v) => Ok(v.unwrap_or(Val::Undef)),
            _ => Err("break or continue outside a loop".into()),
        }
    }

    fn block(&self, stmts: &[Stmt]) -> Result<Flow<'a>, Fail> {
        let mut last = None;
        for s in stmts {
            match self.statement(s)? {
                Flow::Next(Some(v)) => last = Some(v),
                Flow::Next(None) => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Next(last))
    }

    fn statement(&self, s: &Stmt) -> Result<Flow<'a>, Fail> {
        self.step()?;
        match s {
            Stmt::Var(n, init) => {
                let v = match init {
                    Some(e) => self.expr(e)?,
                    None => Val::Undef,
                };
                self.declare(n, v);
                Ok(Flow::Next(None))
            }
            Stmt::Assign(target, op, e) => {
                let v = self.expr(e)?;
                let v = if *op == "=" {
                    v
                } else {
                    let old = self.expr(target)?;
                    self.binary(&op[..1], old, v)?
                };
                self.assign(target, v)?;
                Ok(Flow::Next(None))
            }
            Stmt::If(c, yes, no) => {
                let c = self.host.resolve(self.expr(c)?)?;
                self.block(if c.truthy() { yes } else { no })
            }
            Stmt::For(init, cond, step, body) => {
                self.statement(init)?;
                loop {
                    if !self.host.resolve(self.expr(cond)?)?.truthy() {
                        break;
                    }
                    match self.block(body)? {
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Break => break,
                        _ => {}
                    }
                    self.statement(step)?;
                }
                Ok(Flow::Next(None))
            }
            Stmt::While(cond, body) => {
                while self.host.resolve(self.expr(cond)?)?.truthy() {
                    match self.block(body)? {
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Break => break,
                        _ => {}
                    }
                }
                Ok(Flow::Next(None))
            }
            Stmt::Function(n, ps, body) => {
                self.declare(n, Val::Fun(Rc::new(Fun::User(ps.clone(), body.clone()))));
                Ok(Flow::Next(None))
            }
            Stmt::Return(e) => Ok(Flow::Return(match e {
                Some(e) => self.expr(e)?,
                None => Val::Undef,
            })),
            Stmt::Break => Ok(Flow::Break),
            Stmt::Continue => Ok(Flow::Continue),
            Stmt::Expr(e) => Ok(Flow::Next(Some(self.expr(e)?))),
        }
    }

    fn assign(&self, target: &Expr, v: Val<'a>) -> Result<(), Fail> {
        match target {
            Expr::Name(n) => {
                self.set(n, v);
                Ok(())
            }
            Expr::Index(arr, k) => {
                let Expr::Name(n) = &**arr else { return Err("only a named array's items can be given values".into()) };
                let a = self.lookup(n)?;
                let k = self.number(self.expr(k)?)?;
                let Val::Arr(items) = a else { return Err(format!("{n} is not an array")) };
                let mut items = (*items).clone();
                let i = k.max(0.0) as usize;
                if i >= items.len() {
                    items.resize(i + 1, Val::Undef);
                }
                items[i] = v;
                self.set(n, Val::arr(items));
                Ok(())
            }
            _ => Err("this cannot be given a value".into()),
        }
    }

    pub fn number(&self, v: Val<'a>) -> Result<f64, Fail> {
        match self.host.resolve(v)? {
            Val::Num(n) => Ok(n),
            Val::Bool(b) => Ok(if b { 1.0 } else { 0.0 }),
            other => Err(format!("a number is wanted, not {}", other.kind())),
        }
    }

    pub fn expr(&self, e: &Expr) -> Result<Val<'a>, Fail> {
        self.step()?;
        Ok(match e {
            Expr::Num(n) => Val::Num(*n),
            Expr::Str(s) => Val::Str(s.clone()),
            Expr::Bool(b) => Val::Bool(*b),
            Expr::Name(n) => self.lookup(n)?,
            Expr::Array(items) => Val::arr(items.iter().map(|x| self.expr(x).and_then(|v| self.host.resolve(v))).collect::<Result<_, _>>()?),
            Expr::Unary(op, x) => {
                let v = self.host.resolve(self.expr(x)?)?;
                match *op {
                    "!" => Val::Bool(!v.truthy()),
                    "-" => self.map(v, &|n| -n)?,
                    _ => self.map(v, &|n| n)?,
                }
            }
            Expr::Binary(op, a, b) => {
                if *op == "&&" {
                    let l = self.host.resolve(self.expr(a)?)?;
                    return if l.truthy() { self.expr(b) } else { Ok(l) };
                }
                if *op == "||" {
                    let l = self.host.resolve(self.expr(a)?)?;
                    return if l.truthy() { Ok(l) } else { self.expr(b) };
                }
                let (l, r) = (self.expr(a)?, self.expr(b)?);
                self.binary(op, l, r)?
            }
            Expr::Cond(c, a, b) => {
                if self.host.resolve(self.expr(c)?)?.truthy() {
                    self.expr(a)?
                } else {
                    self.expr(b)?
                }
            }
            Expr::Member(of, name) => {
                let v = self.expr(of)?;
                self.host.member(&v, name)?
            }
            Expr::Index(of, k) => {
                let v = self.host.resolve(self.expr(of)?)?;
                let k = self.host.resolve(self.expr(k)?)?;
                match (&v, &k) {
                    (Val::Arr(items), Val::Num(n)) => items.get(n.max(0.0) as usize).cloned().unwrap_or(Val::Undef),
                    (Val::Str(s), Val::Num(n)) => s.chars().nth(n.max(0.0) as usize).map_or(Val::Undef, |c| Val::Str(c.to_string().into())),
                    (_, Val::Str(name)) => self.host.member(&v, name)?,
                    _ => return Err(format!("{} cannot be indexed by {}", v.kind(), k.kind())),
                }
            }
            Expr::Call(f, args) => {
                // arrays are values: push gives a named array its new items
                if let Expr::Member(of, m) = &**f
                    && &**m == "push"
                    && let Expr::Name(n) = &**of
                    && let Ok(Val::Arr(items)) = self.lookup(n)
                {
                    let mut items = (*items).clone();
                    for a in args {
                        items.push(self.host.resolve(self.expr(a)?)?);
                    }
                    let len = items.len();
                    self.set(n, Val::arr(items));
                    return Ok(Val::Num(len as f64));
                }
                // a method is called on what it belongs to
                let callee = match &**f {
                    Expr::Member(of, name) => {
                        let on = self.expr(of)?;
                        match self.host.member(&on, name) {
                            Ok(Val::Fun(f)) => Val::Fun(f),
                            Ok(other) if matches!(other, Val::Obj(_)) => other,
                            _ => Val::Fun(Rc::new(Fun::Method(on, name.clone()))),
                        }
                    }
                    other => self.expr(other)?,
                };
                let args: Vec<Val<'a>> = args.iter().map(|x| self.expr(x)).collect::<Result<_, _>>()?;
                self.call(callee, args)?
            }
            Expr::Function(ps, body) => Val::Fun(Rc::new(Fun::User(ps.clone(), body.clone()))),
        })
    }

    pub fn call(&self, callee: Val<'a>, args: Vec<Val<'a>>) -> Result<Val<'a>, Fail> {
        match callee {
            Val::Fun(f) => match &*f {
                Fun::User(ps, body) => {
                    if self.scopes.borrow().len() > 64 {
                        return Err("functions call each other too deep".into());
                    }
                    let mut scope = HashMap::new();
                    for (k, p) in ps.iter().enumerate() {
                        scope.insert(p.clone(), args.get(k).cloned().unwrap_or(Val::Undef));
                    }
                    self.scopes.borrow_mut().push(scope);
                    let r = self.program(body);
                    self.scopes.borrow_mut().pop();
                    r
                }
                Fun::Builtin(name) => self.host.call_builtin(self, name, args),
                Fun::Method(on, name) => self.host.call_method(self, on.clone(), name, args),
            },
            // a group or layer called with a name finds what is in it: effect("Blur")("Amount")
            v @ Val::Obj(_) => match args.first() {
                Some(Val::Str(n)) => self.host.member(&v, n),
                Some(Val::Num(k)) => self.host.call_method(self, v, "property", vec![Val::Num(*k)]),
                _ => Err(format!("{} is called with a name", v.kind())),
            },
            other => Err(format!("{} cannot be called", other.kind())),
        }
    }

    /// A number, or each number of an array, through `f`.
    fn map(&self, v: Val<'a>, f: &dyn Fn(f64) -> f64) -> Result<Val<'a>, Fail> {
        match self.host.resolve(v)? {
            Val::Num(n) => Ok(Val::Num(f(n))),
            Val::Arr(items) => Ok(Val::arr(items.iter().map(|x| self.map(x.clone(), f)).collect::<Result<_, _>>()?)),
            Val::Bool(b) => Ok(Val::Num(f(if b { 1.0 } else { 0.0 }))),
            other => Err(format!("{} is not a number", other.kind())),
        }
    }

    pub fn binary(&self, op: &str, l: Val<'a>, r: Val<'a>) -> Result<Val<'a>, Fail> {
        let (l, r) = (self.host.resolve(l)?, self.host.resolve(r)?);
        // strings join with anything
        if op == "+" && (matches!(l, Val::Str(_)) || matches!(r, Val::Str(_))) {
            return Ok(Val::Str(format!("{}{}", show(&l), show(&r)).into()));
        }
        match op {
            "==" | "===" => return Ok(Val::Bool(equal(&l, &r))),
            "!=" | "!==" => return Ok(Val::Bool(!equal(&l, &r))),
            _ => {}
        }
        let num = |v: &Val| match v {
            Val::Num(n) => Some(*n),
            Val::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        };
        let f = |a: f64, b: f64| -> f64 {
            match op {
                "+" => a + b,
                "-" => a - b,
                "*" => a * b,
                "/" => a / b,
                "%" => a % b,
                "**" => a.powf(b),
                _ => f64::NAN,
            }
        };
        if matches!(op, "<" | ">" | "<=" | ">=") {
            let (Some(a), Some(b)) = (num(&l), num(&r)) else { return Err(format!("{} and {} cannot be compared", l.kind(), r.kind())) };
            return Ok(Val::Bool(match op {
                "<" => a < b,
                ">" => a > b,
                "<=" => a <= b,
                _ => a >= b,
            }));
        }
        match (&l, &r) {
            (Val::Arr(a), Val::Arr(b)) => {
                // part by part; the longer array keeps its extra parts (a missing part is 0)
                let n = a.len().max(b.len());
                let part = |v: &Rc<Vec<Val<'a>>>, k: usize| v.get(k).and_then(num);
                Ok(Val::arr(
                    (0..n)
                        .map(|k| match (part(a, k), part(b, k)) {
                            (Some(x), Some(y)) => Val::Num(f(x, y)),
                            (Some(x), None) => Val::Num(if op == "+" || op == "-" { x } else { f(x, 0.0) }),
                            (None, Some(y)) => Val::Num(if op == "+" { y } else if op == "-" { -y } else { f(0.0, y) }),
                            _ => Val::Num(0.0),
                        })
                        .collect(),
                ))
            }
            (Val::Arr(a), _) if num(&r).is_some() => {
                let y = num(&r).unwrap_or(0.0);
                Ok(Val::arr(a.iter().map(|v| Val::Num(f(num(v).unwrap_or(0.0), y))).collect()))
            }
            (_, Val::Arr(b)) if num(&l).is_some() => {
                let x = num(&l).unwrap_or(0.0);
                Ok(Val::arr(b.iter().map(|v| Val::Num(f(x, num(v).unwrap_or(0.0)))).collect()))
            }
            _ => match (num(&l), num(&r)) {
                (Some(a), Some(b)) => Ok(Val::Num(f(a, b))),
                _ => Err(format!("{} {op} {} means nothing", l.kind(), r.kind())),
            },
        }
    }
}

fn equal(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Num(x), Val::Num(y)) => x == y,
        (Val::Str(x), Val::Str(y)) => x == y,
        (Val::Bool(x), Val::Bool(y)) => x == y,
        (Val::Undef, Val::Undef) => true,
        (Val::Arr(x), Val::Arr(y)) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| equal(p, q)),
        _ => false,
    }
}

/// A value as text (joining strings, showing results).
pub fn show(v: &Val) -> String {
    match v {
        Val::Undef => "undefined".into(),
        Val::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                format!("{n}")
            }
        }
        Val::Bool(b) => b.to_string(),
        Val::Str(s) => s.to_string(),
        Val::Arr(items) => items.iter().map(show).collect::<Vec<_>>().join(","),
        Val::Obj(Obj::Layer(l, ..)) => l.name.clone(),
        Val::Obj(_) | Val::Fun(_) => format!("[{}]", v.kind()),
    }
}

/// A child of a group by name: its id, its name for people (any case), or a usual alias.
pub fn child<'g>(g: &'g Group, name: &str) -> Option<&'g Node> {
    let alias = match name {
        "anchorPoint" | "Anchor Point" => "anchor",
        "xRotation" | "X Rotation" => "rotationX",
        "yRotation" | "Y Rotation" => "rotationY",
        "zRotation" | "Z Rotation" => "rotation",
        "xPosition" | "X Position" => "positionX",
        "yPosition" | "Y Position" => "positionY",
        "zPosition" | "Z Position" => "positionZ",
        "cameraOption" => "cameraOptions",
        "lightOption" => "lightOptions",
        other => other,
    };
    let bare = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    g.children
        .iter()
        .find(|c| c.id() == alias)
        .or_else(|| g.children.iter().find(|c| c.name().eq_ignore_ascii_case(name)))
        // maskFeather is "Mask Feather", sourceText "Source Text"
        .or_else(|| g.children.iter().find(|c| bare(c.name()).eq_ignore_ascii_case(&bare(name))))
}
