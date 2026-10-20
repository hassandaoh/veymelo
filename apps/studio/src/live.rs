//! The projects the app holds, the one it shows, and its control channel: an agent bridged with
//! `veymelo mcp --bridge` chooses a project of the same library and works in its very session.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use veymelo_engine::Session;
use veymelo_mcp::{Channel, Library};

struct Live {
    library: Mutex<Arc<Library>>,
    /// The project shown: its file and session.
    shown: Mutex<Option<(PathBuf, Arc<Mutex<Session>>)>>,
    channel: Mutex<Option<Channel>>,
}

static LIVE: OnceLock<Live> = OnceLock::new();

fn live() -> &'static Live {
    LIVE.get_or_init(|| Live { library: Mutex::new(Arc::new(Library::new(veymelo_mcp::library::default_index()))), shown: Mutex::new(None), channel: Mutex::new(None) })
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Open the channel (once). A failure leaves the app working without it.
pub fn open() {
    let l = live();
    let mut channel = lock(&l.channel);
    if channel.is_none() {
        match Channel::open(library(), &veymelo_mcp::default_file()) {
            Ok(c) => *channel = Some(c),
            Err(e) => eprintln!("Veymelo: no control channel ({e}); agents can still use `veymelo mcp` on their own"),
        }
    }
}

/// Close the channel, taking its file away (when the app quits).
pub fn close() {
    if let Some(l) = LIVE.get() {
        lock(&l.channel).take();
    }
}

/// The projects.
pub fn library() -> Arc<Library> {
    lock(&live().library).clone()
}

/// Another list of projects (for tests, before the channel opens).
#[cfg(test)]
pub fn use_library(index: PathBuf) {
    *lock(&live().library) = Arc::new(Library::new(index));
}

/// Show the project in `file` (a project file or its folder): its name.
pub fn show(file: &Path) -> Result<String, String> {
    let (file, session) = library().open(file)?;
    let name = file.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    *lock(&live().shown) = Some((file, session));
    Ok(name)
}

/// Make a project called `name` in a folder of its own inside `folder`, and show it: its name.
pub fn create(name: &str, folder: &Path) -> Result<String, String> {
    let (file, _) = library().create(name, folder)?;
    show(&file)
}

/// Show no project (back on the page of projects).
pub fn hide() {
    *lock(&live().shown) = None;
}

/// The project shown: its file and session.
pub fn shown() -> Option<(PathBuf, Arc<Mutex<Session>>)> {
    lock(&live().shown).clone()
}
