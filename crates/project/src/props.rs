//! A layer's properties as a tree: groups (Transform, Effects, one effect) holding properties
//! (Position, Opacity), each with a value, keyframes or an expression. Properties are found by
//! paths like `transform/position` or `effects/Blur 2/amount`.

use veymelo_json::{Map, Value as Json};
use veymelo_keyframe::{Keyframe, Value, describe, keys_from_json, keys_to_json};
use veymelo_time::Tick;

use crate::defs::{Body, Catalog, DefNode, GroupDef, PropDef};
use crate::read::{LoadError, Obj, fail, join};

/// The id of the group of a composition layer's essential properties.
pub(crate) const ESSENTIALS_ID: &str = "essentials";

/// A formula that works out a property's value.
#[derive(Clone, Debug, PartialEq)]
pub struct Expression {
    pub text: String,
    pub enabled: bool,
}

/// A property and what it holds.
#[derive(Clone, Debug)]
pub struct Property {
    pub def: &'static PropDef,
    /// Its value when it has no keys.
    pub value: Value,
    pub keys: Vec<Keyframe>,
    pub expression: Option<Expression>,
    /// For a property made for something rather than defined in its group (an essential
    /// property of a composition layer): its own id and name.
    pub own: Option<Box<Own>>,
}

/// What a property made for something has of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct Own {
    pub id: u64,
    pub name: String,
    /// Set for this layer (else it follows what it stands for).
    pub overridden: bool,
    /// What a file wrote for it, until what it stands for is known.
    pub pending: Option<Json>,
}

impl PartialEq for Property {
    fn eq(&self, o: &Property) -> bool {
        std::ptr::eq(self.def, o.def) && self.value == o.value && self.keys == o.keys && self.expression == o.expression && self.own == o.own
    }
}

impl Property {
    pub fn new(def: &'static PropDef) -> Property {
        Property { def, value: def.default.clone(), keys: Vec::new(), expression: None, own: None }
    }

    pub fn id(&self) -> &'static str {
        self.def.id
    }

    pub fn is_animated(&self) -> bool {
        !self.keys.is_empty()
    }

    /// Nothing about it differs from its definition.
    pub fn is_default(&self) -> bool {
        self.keys.is_empty() && self.expression.is_none() && self.value == self.def.default
    }

    /// Its value at `t` from its keys (a point along its path), or its one value (expressions
    /// come later).
    pub fn value_at(&self, t: Tick) -> Value {
        veymelo_keyframe::evaluate(&self.keys, t, self.def.spatial).unwrap_or_else(|| self.value.clone())
    }

    /// Give it one value, for all time (its keys go).
    pub fn set_value(&mut self, v: Value) -> Result<(), String> {
        self.def.check(&v)?;
        self.value = v;
        self.keys.clear();
        Ok(())
    }

    /// Give it these keys, in time order.
    pub fn set_keys(&mut self, mut keys: Vec<Keyframe>) -> Result<(), String> {
        if !self.def.animatable && !keys.is_empty() {
            return Err(format!("{} cannot be animated", self.def.id));
        }
        for k in &keys {
            self.def.check(&k.value).map_err(|e| format!("the key at {} s: {e}", k.time.seconds()))?;
        }
        keys.sort_by_key(|k| k.time);
        if let Some(w) = keys.windows(2).find(|w| w[0].time == w[1].time) {
            return Err(format!("two keys at {} s", w[0].time.seconds()));
        }
        if let Some(first) = keys.first() {
            self.value = first.value.clone();
        }
        self.keys = keys;
        Ok(())
    }

    /// As JSON, or nothing when it is as defined. One value is written as itself; keys and
    /// expressions as `{"keys": […], "expression": "…"}`.
    pub fn to_json(&self) -> Option<Json> {
        if self.is_default() {
            return None;
        }
        if self.keys.is_empty() && self.expression.is_none() {
            return Some(self.value.to_json(self.def.kind));
        }
        let mut o = Map::new();
        if self.keys.is_empty() {
            o.insert("value", self.value.to_json(self.def.kind));
        } else {
            o.insert("keys", keys_to_json(&self.keys, self.def.kind));
        }
        if let Some(e) = &self.expression {
            o.insert("expression", e.text.as_str());
            if !e.enabled {
                o.insert("expressionEnabled", false);
            }
        }
        Some(Json::Object(o))
    }

    /// The property `def` with what `j` writes.
    pub fn from_json(def: &'static PropDef, j: &Json, at: &str) -> Result<Property, LoadError> {
        let mut p = Property::new(def);
        let wrapped = matches!(j, Json::Object(m) if ["keys", "value", "expression", "expressionEnabled"].iter().any(|k| m.contains(k)));
        if !wrapped {
            let v = Value::from_json(def.kind, j).map_err(|e| LoadError { at: at.to_string(), message: e })?;
            p.set_value(v).or_else(|e| fail(at, e))?;
            return Ok(p);
        }
        let o = Obj::new(j, at, "a property")?;
        o.only(&["value", "keys", "expression", "expressionEnabled"])?;
        if let Some(v) = o.get("value") {
            if o.get("keys").is_some() {
                return fail(at, "a property with keys has no value of its own (the keys are its value)");
            }
            let v = Value::from_json(def.kind, v).map_err(|e| LoadError { at: o.at("value"), message: e })?;
            p.set_value(v).or_else(|e| fail(&o.at("value"), e))?;
        }
        if let Some(k) = o.get("keys") {
            let keys = keys_from_json(def.kind, k).map_err(|e| LoadError { at: o.at("keys"), message: e })?;
            p.set_keys(keys).or_else(|e| fail(&o.at("keys"), e))?;
        }
        if let Some(text) = o.text("expression")? {
            if !def.animatable {
                return fail(at, format!("{} cannot have an expression", def.id));
            }
            p.expression = Some(Expression { text: text.to_string(), enabled: o.flag("expressionEnabled", true)? });
        } else if o.get("expressionEnabled").is_some() {
            return fail(at, "\"expressionEnabled\" without an \"expression\"");
        }
        Ok(p)
    }
}

/// A group of properties: a fixed one (Transform), a list (Effects), or an instance in a list
/// (one effect, one mask).
#[derive(Clone, Debug)]
pub struct Group {
    pub def: &'static GroupDef,
    /// An instance's id in the project; 0 for fixed groups and lists.
    pub id: u64,
    /// An instance's name ("Mask 1"); empty for fixed groups, which go by their definition's.
    pub name: String,
    /// An instance can be switched off (an effect's switch, a mask's).
    pub enabled: bool,
    pub children: Vec<Node>,
}

impl PartialEq for Group {
    fn eq(&self, o: &Group) -> bool {
        std::ptr::eq(self.def, o.def) && self.id == o.id && self.name == o.name && self.enabled == o.enabled && self.children == o.children
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Prop(Property),
    Group(Group),
}

impl Node {
    pub fn as_prop(&self) -> Option<&Property> {
        if let Node::Prop(p) = self { Some(p) } else { None }
    }

    pub fn as_prop_mut(&mut self) -> Option<&mut Property> {
        if let Node::Prop(p) = self { Some(p) } else { None }
    }

    pub fn as_group(&self) -> Option<&Group> {
        if let Node::Group(g) = self { Some(g) } else { None }
    }

    pub fn as_group_mut(&mut self) -> Option<&mut Group> {
        if let Node::Group(g) = self { Some(g) } else { None }
    }

    /// Its id in paths: a property's or fixed group's id, an instance's type.
    pub fn id(&self) -> &'static str {
        match self {
            Node::Prop(p) => p.def.id,
            Node::Group(g) => g.def.id,
        }
    }

    /// Its name for people.
    pub fn name(&self) -> &str {
        match self {
            Node::Prop(p) => p.own.as_ref().map_or(p.def.name, |o| o.name.as_str()),
            Node::Group(g) => g.name(),
        }
    }
}

impl Group {
    /// A group as defined, every property at its default; a list starts empty.
    pub fn new(def: &'static GroupDef) -> Group {
        let children = match &def.body {
            Body::Fixed(nodes) => nodes
                .iter()
                .map(|n| match n {
                    DefNode::Prop(p) => Node::Prop(Property::new(p)),
                    DefNode::Group(g) => Node::Group(Group::new(g)),
                })
                .collect(),
            Body::List(_) => Vec::new(),
        };
        Group { def, id: 0, name: String::new(), enabled: true, children }
    }

    /// An instance of `def` for a list, with its id and name.
    pub fn instance(def: &'static GroupDef, id: u64, name: &str) -> Group {
        Group { id, name: name.to_string(), ..Group::new(def) }
    }

    pub fn name(&self) -> &str {
        if self.name.is_empty() { self.def.name } else { &self.name }
    }

    pub fn is_list(&self) -> bool {
        matches!(self.def.body, Body::List(_))
    }

    /// Which child `seg` names: an id (`position`), a name (`Blur 2`, any case), the nth
    /// child (`#2`), the nth of a type (`mask#2`) or an instance by id (`@12`).
    fn child(&self, seg: &str) -> Result<usize, String> {
        let found = if let Some(id) = seg.strip_prefix('@') {
            id.parse::<u64>().ok().and_then(|id| {
                self.children.iter().position(|c| match c {
                    Node::Group(g) => g.id == id,
                    Node::Prop(p) => p.own.as_ref().is_some_and(|o| o.id == id),
                })
            })
        } else if let Some(n) = seg.strip_prefix('#') {
            n.parse::<usize>().ok().filter(|n| (1..=self.children.len()).contains(n)).map(|n| n - 1)
        } else {
            let (what, nth) = match seg.rsplit_once('#') {
                Some((what, n)) if n.parse::<usize>().is_ok_and(|n| n >= 1) => (what, n.parse::<usize>().unwrap_or(1)),
                _ => (seg, 1),
            };
            let by_id = self.children.iter().enumerate().filter(|(_, c)| c.id() == what).nth(nth - 1).map(|(i, _)| i);
            by_id.or_else(|| self.children.iter().enumerate().filter(|(_, c)| c.name().eq_ignore_ascii_case(what)).nth(nth - 1).map(|(i, _)| i))
        };
        found.ok_or_else(|| self.missing(seg))
    }

    fn missing(&self, seg: &str) -> String {
        if self.children.is_empty() {
            return format!("{} has no \"{seg}\" (it is empty)", self.def.id);
        }
        let have: Vec<String> = if self.is_list() {
            self.children.iter().enumerate().filter_map(|(i, c)| c.as_group().map(|g| format!("#{} \"{}\" ({}, @{})", i + 1, g.name(), g.def.id, g.id))).collect()
        } else {
            self.children.iter().map(|c| c.id().to_string()).collect()
        };
        format!("{} has no \"{seg}\" (it has {})", self.def.id, have.join(", "))
    }

    /// The node at `path` below this group (`transform/position`).
    pub fn node(&self, path: &str) -> Result<&Node, String> {
        let mut segs = path.split('/').filter(|s| !s.is_empty()).peekable();
        let mut g = self;
        while let Some(seg) = segs.next() {
            let node = &g.children[g.child(seg)?];
            if segs.peek().is_none() {
                return Ok(node);
            }
            g = node.as_group().ok_or_else(|| format!("{} is a property, it has nothing inside", node.id()))?;
        }
        Err("the path is empty".into())
    }

    pub fn node_mut(&mut self, path: &str) -> Result<&mut Node, String> {
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let Some((last, init)) = segs.split_last() else { return Err("the path is empty".into()) };
        let mut g = self;
        for seg in init {
            let i = g.child(seg)?;
            let node = &mut g.children[i];
            let id = node.id();
            g = node.as_group_mut().ok_or_else(|| format!("{id} is a property, it has nothing inside"))?;
        }
        let i = g.child(last)?;
        Ok(&mut g.children[i])
    }

    pub fn prop(&self, path: &str) -> Result<&Property, String> {
        self.node(path)?.as_prop().ok_or_else(|| format!("{path} is a group, not a property"))
    }

    pub fn prop_mut(&mut self, path: &str) -> Result<&mut Property, String> {
        self.node_mut(path)?.as_prop_mut().ok_or_else(|| format!("{path} is a group, not a property"))
    }

    pub fn group(&self, path: &str) -> Result<&Group, String> {
        self.node(path)?.as_group().ok_or_else(|| format!("{path} is a property, not a group"))
    }

    pub fn group_mut(&mut self, path: &str) -> Result<&mut Group, String> {
        self.node_mut(path)?.as_group_mut().ok_or_else(|| format!("{path} is a property, not a group"))
    }

    /// Every property below this group, with its path (instances by id: `masks/@12/path`).
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&str, &'a Property)) {
        self.walk_from("", f);
    }

    fn walk_from<'a>(&'a self, prefix: &str, f: &mut dyn FnMut(&str, &'a Property)) {
        for (i, c) in self.children.iter().enumerate() {
            match c {
                Node::Prop(p) => match &p.own {
                    Some(o) => f(&format!("{prefix}@{}", o.id), p),
                    None => f(&format!("{prefix}{}", p.def.id), p),
                },
                Node::Group(g) => {
                    let seg = match (self.is_list(), g.id) {
                        (false, _) => g.def.id.to_string(),
                        (true, 0) => format!("#{}", i + 1),
                        (true, id) => format!("@{id}"),
                    };
                    g.walk_from(&format!("{prefix}{seg}/"), f);
                }
            }
        }
    }

    /// Every instance in the lists below this group (not the essential properties, which go by
    /// the ids of what they stand for).
    pub fn instances<'a>(&'a self, f: &mut dyn FnMut(&'a Group)) {
        let list = self.is_list();
        for c in &self.children {
            if let Node::Group(g) = c
                && g.def.id != ESSENTIALS_ID
            {
                if list {
                    f(g);
                }
                g.instances(f);
            }
        }
    }

    pub fn instances_mut(&mut self, f: &mut dyn FnMut(&mut Group)) {
        let list = self.is_list();
        for c in &mut self.children {
            if let Node::Group(g) = c
                && g.def.id != ESSENTIALS_ID
            {
                if list {
                    f(g);
                }
                g.instances_mut(f);
            }
        }
    }

    /// What of it differs from its definition, as JSON: an object for a fixed group, a list
    /// for a list; nothing when it is as defined.
    pub fn to_json(&self) -> Option<Json> {
        if self.def.id == ESSENTIALS_ID {
            return crate::essential::overrides_json(self);
        }
        if self.is_list() {
            let items: Vec<Json> = self.children.iter().filter_map(|c| c.as_group().map(Group::instance_json)).collect();
            return (!items.is_empty()).then_some(Json::Array(items));
        }
        let mut o = Map::new();
        for c in &self.children {
            let j = match c {
                Node::Prop(p) => p.to_json(),
                Node::Group(g) => g.to_json(),
            };
            if let Some(j) = j {
                o.insert(c.id(), j);
            }
        }
        (!o.is_empty()).then_some(Json::Object(o))
    }

    fn instance_json(&self) -> Json {
        let mut o = Map::new();
        o.insert("type", self.def.id);
        if self.id != 0 {
            o.insert("id", self.id as f64);
        }
        o.insert("name", self.name());
        if !self.enabled {
            o.insert("enabled", false);
        }
        let inside = Group { id: 0, ..self.clone() };
        if let Some(props) = inside.to_json() {
            o.insert("props", props);
        }
        Json::Object(o)
    }

    /// The group `def` with what `j` writes (nothing: as defined). An instance without an id
    /// gets 0, for the project to give it one.
    pub fn from_json(def: &'static GroupDef, j: Option<&Json>, catalog: &Catalog, at: &str) -> Result<Group, LoadError> {
        let mut g = Group::new(def);
        let Some(j) = j else { return Ok(g) };
        match &def.body {
            Body::List(list) => {
                let Json::Array(items) = j else { return fail(at, format!("{} is a list, got {}", def.id, describe(j))) };
                for (i, item) in items.iter().enumerate() {
                    g.children.push(Node::Group(Group::instance_from_json(list, item, catalog, &format!("{at}[{i}]"))?));
                }
            }
            Body::Fixed(_) => {
                let o = Obj::new(j, at, def.id)?;
                for (key, v) in o.map.iter() {
                    if key == ESSENTIALS_ID && std::ptr::eq(def, &crate::defs::TIMED_LAYER) {
                        g.children.insert(0, Node::Group(crate::essential::pending(v, &join(at, key))?));
                        continue;
                    }
                    let Some(i) = g.children.iter().position(|c| c.id() == key) else {
                        let have: Vec<&str> = g.children.iter().map(Node::id).collect();
                        return fail(at, format!("{} has no \"{key}\" (it has {})", def.id, have.join(", ")));
                    };
                    let here = join(at, key);
                    g.children[i] = match &g.children[i] {
                        Node::Prop(p) => Node::Prop(Property::from_json(p.def, v, &here)?),
                        Node::Group(sub) => Node::Group(Group::from_json(sub.def, Some(v), catalog, &here)?),
                    };
                }
            }
        }
        Ok(g)
    }

    fn instance_from_json(list: &str, j: &Json, catalog: &Catalog, at: &str) -> Result<Group, LoadError> {
        let o = Obj::new(j, at, "an instance")?;
        o.only(&["type", "id", "name", "enabled", "props"])?;
        let ty = o.need_text("type")?;
        let Some(def) = catalog.find(list, ty) else {
            let have: Vec<String> = catalog.types(list).iter().map(|d| format!("\"{}\"", d.id)).collect();
            let have = if have.is_empty() { "none yet".to_string() } else { have.join(", ") };
            return fail(&o.at("type"), format!("{list} have no type \"{ty}\" (there are {have})"));
        };
        let mut g = Group::from_json(def, o.get("props"), catalog, &o.at("props"))?;
        g.id = o.id("id")?.unwrap_or(0);
        g.name = o.text("name")?.unwrap_or(def.name).to_string();
        g.enabled = o.flag("enabled", true)?;
        Ok(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::{MASK, VISUAL_LAYER};
    use veymelo_keyframe::ShapePath;

    fn layer() -> Group {
        Group::new(&VISUAL_LAYER)
    }

    #[test]
    fn a_new_layer_has_its_defaults_and_writes_nothing() {
        let g = layer();
        assert_eq!(g.prop("transform/opacity").unwrap().value, Value::Number(100.0));
        assert_eq!(g.prop("transform/scale").unwrap().value, Value::Vec3([100.0; 3]));
        assert_eq!(g.to_json(), None);
        let mut names = Vec::new();
        g.walk(&mut |path, _| names.push(path.to_string()));
        assert_eq!(names[0], "transform/anchor");
        // transform's 11 and the material options' 9 (styles and the rest are empty lists)
        assert_eq!(names.len(), 20);
    }

    #[test]
    fn paths_find_properties_by_id_name_and_place() {
        let mut g = layer();
        let masks = g.group_mut("masks").unwrap();
        masks.children.push(Node::Group(Group::instance(&MASK, 12, "Mask 1")));
        masks.children.push(Node::Group(Group::instance(&MASK, 15, "Window")));
        assert_eq!(g.prop("transform/position").unwrap().def.name, "Position");
        assert_eq!(g.prop("Transform/Opacity").unwrap().def.id, "opacity");
        assert_eq!(g.group("masks/#2").unwrap().id, 15);
        assert_eq!(g.group("masks/mask#2").unwrap().id, 15);
        assert_eq!(g.group("masks/window").unwrap().id, 15);
        assert_eq!(g.group("masks/@12").unwrap().name, "Mask 1");
        assert_eq!(g.prop("masks/@15/feather").unwrap().def.id, "feather");
        assert_eq!(g.prop("transform/positon").unwrap_err(), "transform has no \"positon\" (it has anchor, position, positionX, positionY, positionZ, scale, orientation, rotationX, rotationY, rotation, opacity)");
        assert_eq!(g.prop("masks/#3").unwrap_err(), "masks has no \"#3\" (it has #1 \"Mask 1\" (mask, @12), #2 \"Window\" (mask, @15))");
        assert_eq!(g.prop("effects/blur").unwrap_err(), "effects has no \"blur\" (it is empty)");
        assert_eq!(g.prop("transform").unwrap_err(), "transform is a group, not a property");
        assert_eq!(g.prop("transform/opacity/x").unwrap_err(), "opacity is a property, it has nothing inside");
        g.prop_mut("masks/window/opacity").unwrap().set_value(Value::Number(40.0)).unwrap();
        assert_eq!(g.prop("masks/@15/opacity").unwrap().value, Value::Number(40.0));
    }

    #[test]
    fn only_what_differs_is_written_and_it_reads_back() {
        let mut g = layer();
        g.prop_mut("transform/position").unwrap().set_value(Value::Vec3([960.0, 540.0, 0.0])).unwrap();
        g.prop_mut("transform/opacity")
            .unwrap()
            .set_keys(vec![Keyframe::new(Tick::ZERO, Value::Number(0.0)), Keyframe::new(Tick::SECOND, Value::Number(100.0)).eased()])
            .unwrap();
        g.prop_mut("transform/rotation").unwrap().expression = Some(Expression { text: "time * 90".into(), enabled: false });
        let mut mask = Group::instance(&MASK, 7, "Mask 1");
        mask.enabled = false;
        mask.prop_mut("path").unwrap().set_value(Value::Path(ShapePath::polygon(&[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]], true))).unwrap();
        mask.prop_mut("mode").unwrap().set_value(Value::Choice(2)).unwrap();
        g.group_mut("masks").unwrap().children.push(Node::Group(mask));
        let j = g.to_json().unwrap();
        assert_eq!(
            j.to_json(),
            concat!(
                r#"{"masks":[{"type":"mask","id":7,"name":"Mask 1","enabled":false,"props":{"mode":"subtract","path":{"points":[[0,0],[100,0],[100,100]],"closed":true}}}],"#,
                r#""transform":{"position":[960,540,0],"rotation":{"value":0,"expression":"time * 90","expressionEnabled":false},"#,
                r#""opacity":{"keys":[{"t":0,"v":0},{"t":1,"v":100,"in":"bezier","out":"bezier"}]}}}"#
            )
        );
        let back = Group::from_json(&VISUAL_LAYER, Some(&j), &Catalog::base(), "props").unwrap();
        assert_eq!(back, g);
    }

    #[test]
    fn written_by_hand() {
        // what an agent might write: a 2D position, an expression, a mask with no id yet
        let text = r#"{"transform": {"position": [100, 200], "opacity": {"value": 50, "expression": "50 + 50 * sin(time)"}},
                       "masks": [{"type": "mask", "props": {"feather": [4, 4]}}]}"#;
        let g = Group::from_json(&VISUAL_LAYER, Some(&Json::parse(text).unwrap()), &Catalog::base(), "props").unwrap();
        assert_eq!(g.prop("transform/position").unwrap().value, Value::Vec3([100.0, 200.0, 0.0]));
        assert_eq!(g.prop("transform/opacity").unwrap().expression.as_ref().map(|e| e.enabled), Some(true));
        let mask = g.group("masks/#1").unwrap();
        assert_eq!((mask.id, mask.name.as_str()), (0, "Mask"));
    }

    #[test]
    fn mistakes_say_where_and_what() {
        let read = |text: &str| Group::from_json(&VISUAL_LAYER, Some(&Json::parse(text).unwrap()), &Catalog::base(), "props").unwrap_err().to_string();
        assert_eq!(read(r#"{"transfrom": {}}"#), "props: layer has no \"transfrom\" (it has masks, effects, transform, layerStyles, materialOptions)");
        assert_eq!(read(r#"{"transform": {"opacity": 150}}"#), "props.transform.opacity: opacity goes from 0 to 100%, not 150");
        assert_eq!(read(r#"{"transform": {"scale": "big"}}"#), "props.transform.scale: expected [x, y, z] (or [x, y], z = 0), got \"big\"");
        assert_eq!(read(r#"{"transform": {"opacity": {"keys": [{"t": 0, "v": 0}, {"t": 1, "v": 200}]}}}"#), "props.transform.opacity.keys: the key at 1 s: opacity goes from 0 to 100%, not 200");
        assert_eq!(read(r#"{"transform": {"opacity": {"value": 1, "keys": []}}}"#), "props.transform.opacity: a property with keys has no value of its own (the keys are its value)");
        assert_eq!(read(r#"{"masks": [{"type": "mask", "props": {"mode": {"keys": [{"t": 0, "v": "add"}]}}}]}"#), "props.masks[0].props.mode.keys: mode cannot be animated");
        assert_eq!(read(r#"{"masks": [{"type": "circle"}]}"#), "props.masks[0].type: masks have no type \"circle\" (there are \"mask\")");
        assert_eq!(read(r#"{"effects": [{"type": "blur"}]}"#), "props.effects[0].type: effects have no type \"blur\" (there are none yet)");
        assert_eq!(read(r#"{"masks": {}}"#), "props.masks: masks is a list, got an object");
        assert_eq!(read(r#"{"masks": [{"type": "mask", "colour": 1}]}"#), "props.masks[0]: there is no \"colour\" here (there is type, id, name, enabled, props)");
    }
}
