//! The window on macOS, made through AppKit straight from Rust (the Objective-C runtime's C
//! functions), with no other library. Its view shows the pixels `studio::paint` draws, and
//! tells `studio` where the pointer is.

use std::ffi::{CString, c_char, c_void};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

type Id = *mut c_void;
type Sel = *mut c_void;
type Class = *mut c_void;

const NIL: Id = std::ptr::null_mut();

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Class;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_allocateClassPair(superclass: Class, name: *const c_char, extra: usize) -> Class;
    fn objc_registerClassPair(class: Class);
    fn class_addMethod(class: Class, name: Sel, imp: *const c_void, types: *const c_char) -> bool;
    fn objc_msgSend();
    #[cfg(target_arch = "x86_64")]
    fn objc_msgSend_stret();
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataCreate(alloc: Id, bytes: *const u8, len: isize) -> Id;
    fn CFRelease(obj: Id);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> Id;
    fn CGDataProviderCreateWithCFData(data: Id) -> Id;
    fn CGImageCreate(width: usize, height: usize, bits: usize, bits_per_pixel: usize, row: usize, space: Id, info: u32, provider: Id, decode: *const f64, interpolate: bool, intent: i32) -> Id;
    fn CGContextDrawImage(ctx: Id, rect: Rect, image: Id);
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn class(name: &str) -> Class {
    let name = CString::new(name).unwrap();
    unsafe { objc_getClass(name.as_ptr()) }
}

fn sel(name: &str) -> Sel {
    let name = CString::new(name).unwrap();
    unsafe { sel_registerName(name.as_ptr()) }
}

/// `objc_msgSend` as a function of the given type (it must match the method called).
macro_rules! msg {
    ($t:ty) => {
        std::mem::transmute::<*const c_void, $t>(objc_msgSend as *const c_void)
    };
}

/// `[obj name]`
unsafe fn send(obj: Id, name: &str) -> Id {
    unsafe { msg!(unsafe extern "C" fn(Id, Sel) -> Id)(obj, sel(name)) }
}

/// `[obj name: arg]`
unsafe fn send_id(obj: Id, name: &str, arg: Id) -> Id {
    unsafe { msg!(unsafe extern "C" fn(Id, Sel, Id) -> Id)(obj, sel(name), arg) }
}

/// `[obj name]` for a method that gives back a rectangle.
unsafe fn send_rect(obj: Id, name: &str) -> Rect {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let mut r = Rect::default();
        std::mem::transmute::<*const c_void, unsafe extern "C" fn(*mut Rect, Id, Sel)>(objc_msgSend_stret as *const c_void)(&mut r, obj, sel(name));
        r
    }
    #[cfg(not(target_arch = "x86_64"))]
    unsafe {
        msg!(unsafe extern "C" fn(Id, Sel) -> Rect)(obj, sel(name))
    }
}

/// An `NSString` of `s`.
unsafe fn string(s: &str) -> Id {
    let s = CString::new(s).unwrap();
    unsafe { msg!(unsafe extern "C" fn(Id, Sel, *const c_char) -> Id)(class("NSString"), sel("stringWithUTF8String:"), s.as_ptr()) }
}

/// The app quits when its window is closed.
extern "C" fn will_quit(_: Id, _: Sel, _: Id) {
    crate::live::close();
}

extern "C" fn quit_when_closed(_: Id, _: Sel, _: Id) -> bool {
    true
}

/// When the window opened.
static OPENED: OnceLock<Instant> = OnceLock::new();
/// The caret's blink when the view last drew.
static DRAWN: Mutex<Option<i64>> = Mutex::new(None);

/// Seconds since the window opened.
fn now() -> f64 {
    OPENED.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// Sixty times a second: the view draws again while something moves, and after that only
/// when the caret blinks.
extern "C" fn tick(this: Id, _: Sel, _timer: Id) {
    let t = now();
    let changed = crate::studio::busy(t) || DRAWN.lock().map_or(true, |d| *d != Some(crate::studio::blink(t)));
    if changed {
        unsafe { msg!(unsafe extern "C" fn(Id, Sel, bool))(this, sel("setNeedsDisplay:"), true) };
    }
}

/// The view draws: our picture at the screen's own pixels, put in its place.
extern "C" fn draw_rect(this: Id, _: Sel, _dirty: Rect) {
    unsafe {
        let bounds = send_rect(this, "bounds");
        let window = send(this, "window");
        let scale = if window.is_null() { 1.0 } else { msg!(unsafe extern "C" fn(Id, Sel) -> f64)(window, sel("backingScaleFactor")) };
        let (w, h) = ((bounds.w * scale).round() as usize, (bounds.h * scale).round() as usize);
        if w == 0 || h == 0 {
            return;
        }
        let t = now();
        if let Ok(mut d) = DRAWN.lock() {
            *d = Some(crate::studio::blink(t));
        }
        let canvas = crate::studio::paint(w, h, scale, t);
        let ctx = send(send(class("NSGraphicsContext"), "currentContext"), "CGContext");
        if ctx.is_null() {
            return;
        }
        let data = CFDataCreate(NIL, canvas.pixels.as_ptr(), canvas.pixels.len() as isize);
        let provider = CGDataProviderCreateWithCFData(data);
        let space = CGColorSpaceCreateDeviceRGB();
        // 8 bits a channel, 4 bytes a pixel, the fourth unused
        let image = CGImageCreate(w, h, 8, 32, w * 4, space, 5, provider, std::ptr::null(), false, 0);
        if !image.is_null() {
            CGContextDrawImage(ctx, bounds, image);
            CFRelease(image);
        }
        CFRelease(space);
        CFRelease(provider);
        CFRelease(data);
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

/// Where the event happened, in points from the view's top left.
unsafe fn place_of(view: Id, event: Id) -> (f64, f64) {
    unsafe {
        let in_window = msg!(unsafe extern "C" fn(Id, Sel) -> Point)(event, sel("locationInWindow"));
        let p = msg!(unsafe extern "C" fn(Id, Sel, Point, Id) -> Point)(view, sel("convertPoint:fromView:"), in_window, NIL);
        (p.x, send_rect(view, "bounds").h - p.y)
    }
}

/// What the pointer looks like: a hand over something to press, a text cursor over a field,
/// else the usual arrow.
unsafe fn cursor(c: crate::workspace::Cursor) {
    use crate::workspace::Cursor;
    let name = match c {
        Cursor::Hand => "pointingHandCursor",
        Cursor::Arrow => "arrowCursor",
        Cursor::UpDown => "resizeUpDownCursor",
    };
    unsafe { send(send(class("NSCursor"), name), "set") };
}

/// The words a key typed.
unsafe fn typed(event: Id) -> String {
    unsafe {
        let s = send(event, "characters");
        if s.is_null() {
            return String::new();
        }
        let p = msg!(unsafe extern "C" fn(Id, Sel) -> *const c_char)(s, sel("UTF8String"));
        if p.is_null() { String::new() } else { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() }
    }
}

/// What a path `NSString` holds.
unsafe fn text_of(s: Id) -> Option<String> {
    unsafe {
        if s.is_null() {
            return None;
        }
        let p = msg!(unsafe extern "C" fn(Id, Sel) -> *const c_char)(s, sel("UTF8String"));
        (!p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
    }
}

/// What the page asked the system for, asked now (outside the page, which waits): a save
/// panel for a new project's name and folder, or an open panel for a project to open; the
/// answer goes back to the page.
unsafe fn ask_system() {
    use crate::workspace::Ask;
    let Some(ask) = crate::studio::take_ask() else { return };
    if ask == Ask::Reveal {
        // the page is not held while Finder comes forward (it may make the window draw)
        if let Some(path) = crate::studio::take_revealing() {
            reveal(&path);
        }
        return;
    }
    if ask == Ask::Copy {
        if let Some(words) = crate::studio::take_copying() {
            unsafe { copy_text(&words) };
        }
        return;
    }
    if ask == Ask::OpenFile {
        if let Some(path) = crate::studio::take_opening() {
            open(&path);
        }
        return;
    }
    if ask == Ask::SetUp {
        if let Some(app) = crate::studio::take_setting_up()
            && unsafe { ask_set_up(app) }
        {
            crate::setup::start(app);
        }
        return;
    }
    if ask == Ask::DeleteProject {
        let choice = unsafe { ask_delete(&crate::studio::deleting_name().unwrap_or_default()) };
        // moved to the Trash here, with the page not held
        let trashed = (choice == crate::workspace::Delete::Trash).then(|| crate::studio::trash_target().and_then(|what| trash(&what)));
        crate::studio::answer_delete(choice, trashed, now());
        return;
    }
    let yes = |obj: Id, name: &str| unsafe { msg!(unsafe extern "C" fn(Id, Sel, bool))(obj, sel(name), true) };
    let chosen = unsafe {
        let panel = match ask {
            Ask::RenderTo => {
                let p = send(class("NSSavePanel"), "savePanel");
                let (name, dir) = crate::studio::render_defaults().unwrap_or_else(|| ("Render.mp4".into(), std::path::PathBuf::new()));
                send_id(p, "setTitle:", string("Render"));
                send_id(p, "setMessage:", string("Where the video goes: an MP4 of the composition shown."));
                send_id(p, "setPrompt:", string("Render"));
                send_id(p, "setNameFieldStringValue:", string(&name));
                if !dir.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(&dir);
                    send_id(p, "setDirectoryURL:", send_id(class("NSURL"), "fileURLWithPath:", string(&dir.display().to_string())));
                }
                yes(p, "setCanCreateDirectories:");
                p
            }
            Ask::NewProject => {
                let p = send(class("NSSavePanel"), "savePanel");
                send_id(p, "setTitle:", string("New project"));
                send_id(p, "setMessage:", string("Name the project and choose where it goes: it gets a folder of its own there."));
                send_id(p, "setPrompt:", string("Create"));
                send_id(p, "setNameFieldStringValue:", string("Untitled"));
                yes(p, "setCanCreateDirectories:");
                p
            }
            Ask::DeleteProject | Ask::Reveal | Ask::Copy | Ask::SetUp | Ask::OpenFile => return,
            Ask::OpenProject => {
                let p = send(class("NSOpenPanel"), "openPanel");
                send_id(p, "setMessage:", string("Choose a project's folder, or its .veymelo file."));
                send_id(p, "setPrompt:", string("Open"));
                yes(p, "setCanChooseDirectories:");
                yes(p, "setCanChooseFiles:");
                p
            }
        };
        let answer = msg!(unsafe extern "C" fn(Id, Sel) -> isize)(panel, sel("runModal"));
        if answer == 1 { text_of(send(send(panel, "URL"), "path")) } else { None }
    };
    crate::studio::answer(ask, chosen.map(std::path::PathBuf::from), now());
}

/// Puts `words` on the clipboard.
unsafe fn copy_text(words: &str) {
    unsafe {
        let board = send(class("NSPasteboard"), "generalPasteboard");
        send(board, "clearContents");
        msg!(unsafe extern "C" fn(Id, Sel, Id, Id) -> bool)(board, sel("setString:forType:"), string(words), string("public.utf8-plain-text"));
    }
}

/// Asks whether to set `app` up to reach Veymelo, saying what changes: yes or no.
unsafe fn ask_set_up(app: crate::setup::App) -> bool {
    use crate::setup::App;
    let (title, words) = match app {
        App::Code => (
            "Set up Veymelo in Claude Code?",
            format!("Claude Code's own tool adds Veymelo for all your projects:\n\n{}\n\nA server already named \u{201c}veymelo\u{201d} is replaced. New Claude Code sessions will have it.", crate::setup::code_line()),
        ),
        App::Desktop => (
            "Set up Veymelo in Claude Desktop?",
            "Veymelo is added to Claude Desktop's settings (claude_desktop_config.json), and a copy of the file as it was is kept beside it. Quit Claude Desktop and open it again afterwards to use it.".to_string(),
        ),
    };
    unsafe {
        let alert = send(send(class("NSAlert"), "alloc"), "init");
        send_id(alert, "setMessageText:", string(title));
        send_id(alert, "setInformativeText:", string(&words));
        send_id(alert, "addButtonWithTitle:", string("Set Up"));
        let cancel = send_id(alert, "addButtonWithTitle:", string("Cancel"));
        send_id(cancel, "setKeyEquivalent:", string("\u{1b}"));
        msg!(unsafe extern "C" fn(Id, Sel) -> isize)(alert, sel("runModal")) == 1000
    }
}

/// Asks what to do with the project `name` being deleted: take it off the list only, also move
/// it to the Trash, or nothing (Escape).
unsafe fn ask_delete(name: &str) -> crate::workspace::Delete {
    use crate::workspace::Delete;
    unsafe {
        let alert = send(send(class("NSAlert"), "alloc"), "init");
        send_id(alert, "setMessageText:", string(&format!("Delete \u{201c}{name}\u{201d}?")));
        send_id(alert, "setInformativeText:", string("Take it off the list only (its folder stays where it is), or also move its folder to the Trash (it can be put back from there)."));
        send_id(alert, "addButtonWithTitle:", string("Remove from List"));
        send_id(alert, "addButtonWithTitle:", string("Move to Trash"));
        let cancel = send_id(alert, "addButtonWithTitle:", string("Cancel"));
        send_id(cancel, "setKeyEquivalent:", string("\u{1b}"));
        match msg!(unsafe extern "C" fn(Id, Sel) -> isize)(alert, sel("runModal")) {
            1000 => Delete::Remove,
            1001 => Delete::Trash,
            _ => Delete::Cancel,
        }
    }
}

/// Shows the file at `path` in a Finder window, chosen.
pub fn reveal(path: &std::path::Path) {
    unsafe {
        let url = send_id(class("NSURL"), "fileURLWithPath:", string(&path.display().to_string()));
        let list = send_id(class("NSArray"), "arrayWithObject:", url);
        send_id(send(class("NSWorkspace"), "sharedWorkspace"), "activateFileViewerSelectingURLs:", list);
    }
}

/// Opens the file at `path` in its own app (as Finder would).
pub fn open(path: &std::path::Path) {
    unsafe {
        let url = send_id(class("NSURL"), "fileURLWithPath:", string(&path.display().to_string()));
        msg!(unsafe extern "C" fn(Id, Sel, Id) -> bool)(send(class("NSWorkspace"), "sharedWorkspace"), sel("openURL:"), url);
    }
}

/// The sound playing from the project's files (an `NSSound`), if one is.
static SOUND: Mutex<usize> = Mutex::new(0);

/// Plays the sound file at `path` (stopping the one playing): whether it could.
pub fn play_sound(path: &std::path::Path) -> bool {
    stop_sound();
    unsafe {
        let s = msg!(unsafe extern "C" fn(Id, Sel, Id, bool) -> Id)(send(class("NSSound"), "alloc"), sel("initWithContentsOfFile:byReference:"), string(&path.display().to_string()), true);
        if s.is_null() {
            return false;
        }
        let played = msg!(unsafe extern "C" fn(Id, Sel) -> bool)(s, sel("play"));
        *SOUND.lock().unwrap_or_else(|e| e.into_inner()) = s as usize;
        played
    }
}

/// Stops the sound file playing, if one is.
pub fn stop_sound() {
    let s = std::mem::take(&mut *SOUND.lock().unwrap_or_else(|e| e.into_inner())) as Id;
    if !s.is_null() {
        unsafe {
            msg!(unsafe extern "C" fn(Id, Sel) -> bool)(s, sel("stop"));
            send(s, "release");
        }
    }
}

/// Whether a sound file is playing still.
pub fn sound_playing() -> bool {
    let s = *SOUND.lock().unwrap_or_else(|e| e.into_inner()) as Id;
    !s.is_null() && unsafe { msg!(unsafe extern "C" fn(Id, Sel) -> bool)(s, sel("isPlaying")) }
}

/// Files dragged over the view from Finder: they can be let go (copied).
extern "C" fn dragging_entered(_: Id, _: Sel, _: Id) -> u64 {
    1
}

/// Files let go over the view: where, and their paths, go to the studio.
extern "C" fn perform_drag(this: Id, _: Sel, info: Id) -> bool {
    unsafe {
        let in_window = msg!(unsafe extern "C" fn(Id, Sel) -> Point)(info, sel("draggingLocation"));
        let p = msg!(unsafe extern "C" fn(Id, Sel, Point, Id) -> Point)(this, sel("convertPoint:fromView:"), in_window, NIL);
        let at = (p.x, send_rect(this, "bounds").h - p.y);
        let board = send(info, "draggingPasteboard");
        let classes = send_id(class("NSArray"), "arrayWithObject:", class("NSURL"));
        let urls = msg!(unsafe extern "C" fn(Id, Sel, Id, Id) -> Id)(board, sel("readObjectsForClasses:options:"), classes, NIL);
        if urls.is_null() {
            return false;
        }
        let n = msg!(unsafe extern "C" fn(Id, Sel) -> usize)(urls, sel("count"));
        let paths: Vec<std::path::PathBuf> = (0..n).filter_map(|i| text_of(send(msg!(unsafe extern "C" fn(Id, Sel, usize) -> Id)(urls, sel("objectAtIndex:"), i), "path"))).map(std::path::PathBuf::from).collect();
        crate::studio::dropped(&paths, at, now());
        ask_system();
        redraw(this);
        !paths.is_empty()
    }
}

/// Projects Finder asks the app to open: double-clicked, opened with Veymelo, or let go on its
/// icon in the Dock (also the one it was started for).
extern "C" fn open_urls(_: Id, _: Sel, _: Id, urls: Id) {
    unsafe {
        let n = msg!(unsafe extern "C" fn(Id, Sel) -> usize)(urls, sel("count"));
        let paths: Vec<std::path::PathBuf> = (0..n).filter_map(|i| text_of(send(msg!(unsafe extern "C" fn(Id, Sel, usize) -> Id)(urls, sel("objectAtIndex:"), i), "path"))).map(std::path::PathBuf::from).collect();
        crate::studio::opened(&paths, now());
    }
}

/// Moves the file or folder at `path` to the Trash (it can be put back from there).
pub fn trash(path: &std::path::Path) -> Result<(), String> {
    unsafe {
        let manager = send(class("NSFileManager"), "defaultManager");
        let url = send_id(class("NSURL"), "fileURLWithPath:", string(&path.display().to_string()));
        let mut error: Id = NIL;
        let ok = msg!(unsafe extern "C" fn(Id, Sel, Id, *mut Id, *mut Id) -> bool)(manager, sel("trashItemAtURL:resultingItemURL:error:"), url, std::ptr::null_mut(), &mut error);
        if ok {
            Ok(())
        } else {
            Err(if error.is_null() { "the system said no".into() } else { text_of(send(error, "localizedDescription")).unwrap_or_default() })
        }
    }
}

unsafe fn redraw(view: Id) {
    unsafe { msg!(unsafe extern "C" fn(Id, Sel, bool))(view, sel("setNeedsDisplay:"), true) };
}

/// The pointer moved over the view (or was dragged across it).
extern "C" fn mouse_moved(this: Id, _: Sel, event: Id) {
    unsafe {
        cursor(crate::studio::pointer(Some(place_of(this, event)), now()));
        redraw(this);
    }
}

/// The pointer left the view.
extern "C" fn mouse_exited(this: Id, _: Sel, _event: Id) {
    crate::studio::pointer(None, now());
    unsafe {
        cursor(crate::workspace::Cursor::Arrow);
        redraw(this);
    }
}

/// The mouse button went down, or came up.
extern "C" fn mouse_down(this: Id, _: Sel, event: Id) {
    unsafe {
        crate::studio::press(true, place_of(this, event), now());
        cursor(crate::studio::cursor_now());
        redraw(this);
    }
}

extern "C" fn mouse_up(this: Id, _: Sel, event: Id) {
    unsafe {
        crate::studio::press(false, place_of(this, event), now());
        ask_system();
        cursor(crate::studio::cursor_now());
        redraw(this);
    }
}

/// The pointer's wheel or two fingers scrolled: how far (points; a wheel's lines made points).
extern "C" fn scroll_wheel(this: Id, _: Sel, event: Id) {
    unsafe {
        let f = |name: &str| msg!(unsafe extern "C" fn(Id, Sel) -> f64)(event, sel(name));
        let precise = msg!(unsafe extern "C" fn(Id, Sel) -> bool)(event, sel("hasPreciseScrollingDeltas"));
        let k = if precise { 1.0 } else { 18.0 };
        crate::studio::scroll((-f("scrollingDeltaX") * k, -f("scrollingDeltaY") * k), place_of(this, event), now());
        redraw(this);
    }
}

/// Two fingers pinched: by how much (above 0: apart).
extern "C" fn magnify(this: Id, _: Sel, event: Id) {
    unsafe {
        let k = msg!(unsafe extern "C" fn(Id, Sel) -> f64)(event, sel("magnification"));
        crate::studio::magnify(k, place_of(this, event));
        redraw(this);
    }
}

/// A key: its code, what it types and whether Shift is down go to the studio.
extern "C" fn key_down(this: Id, _: Sel, event: Id) {
    unsafe {
        let code = msg!(unsafe extern "C" fn(Id, Sel) -> u16)(event, sel("keyCode"));
        let shift = msg!(unsafe extern "C" fn(Id, Sel) -> u64)(event, sel("modifierFlags")) & SHIFT != 0;
        crate::studio::key_with(code, &typed(event), shift, now());
        ask_system();
        cursor(crate::studio::cursor_now());
        redraw(this);
    }
}

/// What a menu item of ours asks (the window draws again on its next tick).
fn command(c: crate::workspace::Command) {
    crate::studio::command(c, now());
    unsafe { ask_system() };
}

extern "C" fn new_project(_: Id, _: Sel, _: Id) {
    command(crate::workspace::Command::NewProject);
}
extern "C" fn open_project(_: Id, _: Sel, _: Id) {
    command(crate::workspace::Command::Open);
}
extern "C" fn render_shown(_: Id, _: Sel, _: Id) {
    command(crate::workspace::Command::Render);
}
extern "C" fn back_out(_: Id, _: Sel, _: Id) {
    command(crate::workspace::Command::Back);
}
extern "C" fn zoom_in(_: Id, _: Sel, _: Id) {
    command(crate::workspace::Command::ZoomIn);
}
extern "C" fn zoom_out(_: Id, _: Sel, _: Id) {
    command(crate::workspace::Command::ZoomOut);
}
extern "C" fn zoom_all(_: Id, _: Sel, _: Id) {
    command(crate::workspace::Command::ZoomAll);
}
extern "C" fn toggle_steps(_: Id, _: Sel, _: Id) {
    command(crate::workspace::Command::TogglePanel);
}
extern "C" fn toggle_files(_: Id, _: Sel, _: Id) {
    command(crate::workspace::Command::ToggleFiles);
}
extern "C" fn show_in_finder(_: Id, _: Sel, _: Id) {
    command(crate::workspace::Command::Reveal);
}
extern "C" fn comp_settings(_: Id, _: Sel, _: Id) {
    command(crate::workspace::Command::Settings);
}

/// Paste: the clipboard's words go to the field typed in.
extern "C" fn paste(_: Id, _: Sel, _: Id) {
    let words = unsafe { text_of(msg!(unsafe extern "C" fn(Id, Sel, Id) -> Id)(send(class("NSPasteboard"), "generalPasteboard"), sel("stringForType:"), string("public.utf8-plain-text"))) };
    if let Some(words) = words {
        crate::studio::paste(&words);
    }
}

/// Select All: all of the field typed in.
extern "C" fn select_all(_: Id, _: Sel, _: Id) {
    crate::studio::select_all();
}

/// The modifier keys' flags, as menus and key events tell them.
const SHIFT: u64 = 1 << 17;
const CONTROL: u64 = 1 << 18;
const OPTION: u64 = 1 << 19;
const COMMAND: u64 = 1 << 20;

/// A menu named `title` on the menu bar `bar`.
unsafe fn menu(bar: Id, title: &str) -> Id {
    unsafe {
        let item = send(send(class("NSMenuItem"), "alloc"), "init");
        send_id(bar, "addItem:", item);
        let m = send_id(send(class("NSMenu"), "alloc"), "initWithTitle:", string(title));
        send_id(item, "setSubmenu:", m);
        m
    }
}

/// An item of `menu`: its title, the action it sends (to `target`, else along the responder
/// chain), its key with the modifier keys it takes.
unsafe fn item(menu: Id, title: &str, action: &str, key: &str, mods: u64, target: Id) {
    unsafe {
        let it = msg!(unsafe extern "C" fn(Id, Sel, Id, Sel, Id) -> Id)(send(class("NSMenuItem"), "alloc"), sel("initWithTitle:action:keyEquivalent:"), string(title), sel(action), string(key));
        if !key.is_empty() && mods != COMMAND {
            msg!(unsafe extern "C" fn(Id, Sel, u64))(it, sel("setKeyEquivalentModifierMask:"), mods);
        }
        if !target.is_null() {
            send_id(it, "setTarget:", target);
        }
        send_id(menu, "addItem:", it);
    }
}

unsafe fn separator(menu: Id) {
    unsafe { send_id(menu, "addItem:", send(class("NSMenuItem"), "separatorItem")) };
}

/// The menu bar: the app's menu, File, Edit (for the system's panels' text), View and Window.
unsafe fn menus(app: Id, ours: Id) {
    unsafe {
        let bar = send(send(class("NSMenu"), "alloc"), "init");
        let m = menu(bar, "Veymelo");
        item(m, "About Veymelo", "orderFrontStandardAboutPanel:", "", 0, NIL);
        separator(m);
        item(m, "Hide Veymelo", "hide:", "h", COMMAND, NIL);
        item(m, "Hide Others", "hideOtherApplications:", "h", COMMAND | OPTION, NIL);
        item(m, "Show All", "unhideAllApplications:", "", 0, NIL);
        separator(m);
        item(m, "Quit Veymelo", "terminate:", "q", COMMAND, NIL);
        let m = menu(bar, "File");
        item(m, "New Project…", "newProject:", "n", COMMAND, ours);
        item(m, "Open…", "openProject:", "o", COMMAND, ours);
        separator(m);
        item(m, "Render…", "renderShown:", "r", COMMAND, ours);
        item(m, "Show in Finder", "showInFinder:", "r", COMMAND | SHIFT, ours);
        item(m, "Composition Settings…", "compSettings:", ",", COMMAND, ours);
        separator(m);
        item(m, "Back to Projects", "backOut:", "[", COMMAND, ours);
        item(m, "Close Window", "performClose:", "w", COMMAND, NIL);
        let m = menu(bar, "Edit");
        item(m, "Undo", "undo:", "z", COMMAND, NIL);
        item(m, "Redo", "redo:", "z", COMMAND | SHIFT, NIL);
        separator(m);
        item(m, "Cut", "cut:", "x", COMMAND, NIL);
        item(m, "Copy", "copy:", "c", COMMAND, NIL);
        item(m, "Paste", "paste:", "v", COMMAND, NIL);
        item(m, "Select All", "selectAll:", "a", COMMAND, NIL);
        let m = menu(bar, "View");
        item(m, "Zoom In", "zoomIn:", "=", COMMAND, ours);
        item(m, "Zoom Out", "zoomOut:", "-", COMMAND, ours);
        item(m, "Whole Timeline", "zoomAll:", "0", COMMAND, ours);
        separator(m);
        item(m, "Show/Hide Files", "toggleFiles:", "\\", COMMAND | SHIFT, ours);
        item(m, "Show/Hide AI Steps", "toggleSteps:", "\\", COMMAND, ours);
        separator(m);
        item(m, "Enter Full Screen", "toggleFullScreen:", "f", COMMAND | CONTROL, NIL);
        let m = menu(bar, "Window");
        item(m, "Minimize", "performMiniaturize:", "m", COMMAND, NIL);
        item(m, "Zoom", "performZoom:", "", 0, NIL);
        send_id(app, "setWindowsMenu:", m);
        send_id(app, "setMainMenu:", bar);
    }
}

/// The view takes keys, and a click that brings the window forward also counts.
extern "C" fn yes(_: Id, _: Sel) -> bool {
    true
}

extern "C" fn yes_for(_: Id, _: Sel, _: Id) -> bool {
    true
}

/// The view covers the window completely.
extern "C" fn is_opaque(_: Id, _: Sel) -> bool {
    true
}

/// A class of our own: `superclass` with these methods (selector, function, type codes).
unsafe fn subclass(name: &str, superclass: &str, methods: &[(&str, *const c_void, &str)]) -> Class {
    let name = CString::new(name).unwrap();
    unsafe {
        let c = objc_allocateClassPair(class(superclass), name.as_ptr(), 0);
        for (selector, imp, types) in methods {
            let types = CString::new(*types).unwrap();
            class_addMethod(c, sel(selector), *imp, types.as_ptr());
        }
        objc_registerClassPair(c);
        c
    }
}

pub fn run() {
    unsafe {
        let _pool = send(send(class("NSAutoreleasePool"), "alloc"), "init");
        let app = send(class("NSApplication"), "sharedApplication");
        // a regular app: in the Dock, with a menu bar
        msg!(unsafe extern "C" fn(Id, Sel, i64) -> bool)(app, sel("setActivationPolicy:"), 0);

        // its delegate, so closing the window quits it; it also answers our menu items
        let delegate_class = subclass(
            "VeymeloDelegate",
            "NSObject",
            &[
                ("applicationShouldTerminateAfterLastWindowClosed:", quit_when_closed as *const c_void, "B@:@"),
                ("applicationWillTerminate:", will_quit as *const c_void, "v@:@"),
                ("application:openURLs:", open_urls as *const c_void, "v@:@@"),
                ("newProject:", new_project as *const c_void, "v@:@"),
                ("openProject:", open_project as *const c_void, "v@:@"),
                ("renderShown:", render_shown as *const c_void, "v@:@"),
                ("backOut:", back_out as *const c_void, "v@:@"),
                ("zoomIn:", zoom_in as *const c_void, "v@:@"),
                ("zoomOut:", zoom_out as *const c_void, "v@:@"),
                ("zoomAll:", zoom_all as *const c_void, "v@:@"),
                ("toggleSteps:", toggle_steps as *const c_void, "v@:@"),
                ("toggleFiles:", toggle_files as *const c_void, "v@:@"),
                ("showInFinder:", show_in_finder as *const c_void, "v@:@"),
                ("compSettings:", comp_settings as *const c_void, "v@:@"),
            ],
        );
        let delegate = send(send(delegate_class, "alloc"), "init");
        send_id(app, "setDelegate:", delegate);
        menus(app, delegate);

        // the window, and our view in it
        let frame = Rect { x: 0.0, y: 0.0, w: 1280.0, h: 800.0 };
        let style: u64 = 1 | 2 | 4 | 8; // titled, closable, miniaturizable, resizable
        let window = msg!(unsafe extern "C" fn(Id, Sel, Rect, u64, u64, bool) -> Id)(send(class("NSWindow"), "alloc"), sel("initWithContentRect:styleMask:backing:defer:"), frame, style, 2, false);
        send_id(window, "setTitle:", string("Veymelo"));
        send_id(window, "setAppearance:", send_id(class("NSAppearance"), "appearanceNamed:", string("NSAppearanceNameAqua")));
        send_id(window, "setBackgroundColor:", send(class("NSColor"), "whiteColor"));
        let view_class = subclass(
            "VeymeloView",
            "NSView",
            &[
                ("drawRect:", draw_rect as *const c_void, "v@:{CGRect={CGPoint=dd}{CGSize=dd}}"),
                ("isOpaque", is_opaque as *const c_void, "B@:"),
                ("tick:", tick as *const c_void, "v@:@"),
                ("mouseMoved:", mouse_moved as *const c_void, "v@:@"),
                ("mouseDragged:", mouse_moved as *const c_void, "v@:@"),
                ("mouseEntered:", mouse_moved as *const c_void, "v@:@"),
                ("mouseExited:", mouse_exited as *const c_void, "v@:@"),
                ("mouseDown:", mouse_down as *const c_void, "v@:@"),
                ("mouseUp:", mouse_up as *const c_void, "v@:@"),
                ("keyDown:", key_down as *const c_void, "v@:@"),
                ("paste:", paste as *const c_void, "v@:@"),
                ("selectAll:", select_all as *const c_void, "v@:@"),
                ("scrollWheel:", scroll_wheel as *const c_void, "v@:@"),
                ("magnifyWithEvent:", magnify as *const c_void, "v@:@"),
                ("acceptsFirstResponder", yes as *const c_void, "B@:"),
                ("acceptsFirstMouse:", yes_for as *const c_void, "B@:@"),
                ("draggingEntered:", dragging_entered as *const c_void, "Q@:@"),
                ("draggingUpdated:", dragging_entered as *const c_void, "Q@:@"),
                ("prepareForDragOperation:", yes_for as *const c_void, "B@:@"),
                ("performDragOperation:", perform_drag as *const c_void, "B@:@"),
            ],
        );
        let view = msg!(unsafe extern "C" fn(Id, Sel, Rect) -> Id)(send(view_class, "alloc"), sel("initWithFrame:"), frame);
        send_id(window, "setContentView:", view);
        // files from Finder can be let go on it; and it keeps room for the project's columns
        send_id(view, "registerForDraggedTypes:", send_id(class("NSArray"), "arrayWithObject:", string("public.file-url")));
        msg!(unsafe extern "C" fn(Id, Sel, f64, f64))(window, sel("setContentMinSize:"), 1040.0, 660.0);
        // the pointer, over the whole view whatever its size, whether the app is in front or not
        let options: u64 = 0x01 | 0x02 | 0x80 | 0x200; // entered and exited, moved, always, the visible part
        let area = msg!(unsafe extern "C" fn(Id, Sel, Rect, u64, Id, Id) -> Id)(send(class("NSTrackingArea"), "alloc"), sel("initWithRect:options:owner:userInfo:"), Rect::default(), options, view, NIL);
        send_id(view, "addTrackingArea:", area);
        msg!(unsafe extern "C" fn(Id, Sel, bool))(window, sel("setAcceptsMouseMovedEvents:"), true);
        send_id(window, "makeFirstResponder:", view);
        send(window, "center");
        send_id(window, "makeKeyAndOrderFront:", NIL);
        // the clock: from now, sixty ticks a second, also while the window is being resized
        OPENED.get_or_init(Instant::now);
        let timer = msg!(unsafe extern "C" fn(Id, Sel, f64, Id, Sel, Id, bool) -> Id)(class("NSTimer"), sel("timerWithTimeInterval:target:selector:userInfo:repeats:"), 1.0 / 60.0, view, sel("tick:"), NIL, true);
        msg!(unsafe extern "C" fn(Id, Sel, Id, Id))(send(class("NSRunLoop"), "currentRunLoop"), sel("addTimer:forMode:"), timer, string("kCFRunLoopCommonModes"));
        msg!(unsafe extern "C" fn(Id, Sel, bool))(app, sel("activateIgnoringOtherApps:"), true);
        send(app, "run");
    }
}
