//! What the window keeps between runs: which columns are folded, which folders of a project
//! are open. A small JSON file beside the list of projects (Application Support/Veymelo), read
//! once and written whenever something in it changes. A file that cannot be read is as good as
//! none.

use std::path::PathBuf;
use std::sync::Mutex;

use veymelo_json::{Map, Value};

/// Where it is kept: `$VEYMELO_PREFS`, or `studio.json` in the app's folder.
fn file() -> PathBuf {
    if let Some(p) = std::env::var_os("VEYMELO_PREFS") {
        return PathBuf::from(p);
    }
    // tests never touch the person's own
    if cfg!(test) {
        return std::env::temp_dir().join(format!("veymelo-prefs-{}.json", std::process::id()));
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join("Library/Application Support/Veymelo/studio.json")
}

static KEPT: Mutex<Option<Map>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Map) -> T) -> T {
    let mut g = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(|| {
        std::fs::read_to_string(file()).ok().and_then(|t| Value::parse(&t).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
    });
    f(map)
}

fn save(map: &Map) {
    let path = file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, Value::Object(map.clone()).to_pretty());
}

/// A yes or no kept under `key` (`or` when there is none).
pub fn flag(key: &str, or: bool) -> bool {
    with(|m| m.get(key).and_then(Value::as_bool).unwrap_or(or))
}

pub fn set_flag(key: &str, on: bool) {
    with(|m| {
        if m.get(key).and_then(Value::as_bool) != Some(on) {
            m.insert(key, Value::Bool(on));
            save(m);
        }
    });
}

/// A list of words kept under `key` (none when there is none).
pub fn list(key: &str) -> Option<Vec<String>> {
    with(|m| m.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()))
}

pub fn set_list(key: &str, words: &[String]) {
    with(|m| {
        let v = Value::Array(words.iter().map(|w| Value::from(w.as_str())).collect());
        if m.get(key) != Some(&v) {
            m.insert(key, v);
            save(m);
        }
    });
}
