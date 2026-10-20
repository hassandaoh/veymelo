//! Veymelo: a white window where a grid of dots comes out, "hello" and "welcome to veymelo
//! studio" are typed one letter at a time, and a button goes into the workspace.

mod button;
mod connect;
mod export;
mod faces;
mod files;
mod icon;
mod iso;
mod live;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
mod sound;
mod motion;
mod prefs;
mod renders;
mod settings;
mod setup;
mod studio;
mod thumbs;
mod timeline;
mod watch;
mod workspace;

/// Elsewhere: no Trash and no sound yet.
#[cfg(not(target_os = "macos"))]
mod mac {
    pub fn trash(_: &std::path::Path) -> Result<(), String> {
        Err("the Trash is only on macOS for now".into())
    }
    pub fn reveal(_: &std::path::Path) {}
    pub fn open(_: &std::path::Path) {}
    pub fn play_sound(_: &std::path::Path) -> bool {
        false
    }
    pub fn stop_sound() {}
    pub fn sound_playing() -> bool {
        false
    }
}
#[cfg(not(target_os = "macos"))]
mod sound {
    pub fn play(_: std::sync::Arc<veymelo_engine::project::Project>, _: u64, _: f64) {}
    pub fn stop() {}
    pub fn clock() -> Option<f64> {
        None
    }
}

fn main() {
    // `--icon FOLDER`: the app's icon at every size, for the app's bundle (scripts/bundle.sh)
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, dir] = args.as_slice()
        && flag == "--icon"
    {
        if let Err(e) = icon::write_set(std::path::Path::new(dir)) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    live::open();
    #[cfg(target_os = "macos")]
    mac::run();
    live::close();
    #[cfg(not(target_os = "macos"))]
    eprintln!("Veymelo opens on macOS for now");
}
