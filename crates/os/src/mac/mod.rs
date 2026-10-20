//! macOS: the system's frameworks called straight through the Objective-C runtime's C
//! functions and the frameworks' C functions (no other library).

#![allow(non_upper_case_globals)]

pub mod asset;
pub mod gpu;
pub mod picture;
pub mod write;

use std::ffi::{CString, c_char, c_void};

pub type Id = *mut c_void;
pub type Sel = *mut c_void;

pub const NIL: Id = std::ptr::null_mut();

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    pub fn objc_msgSend();
    pub fn objc_autoreleasePoolPush() -> *mut c_void;
    pub fn objc_autoreleasePoolPop(pool: *mut c_void);
    fn objc_setExceptionPreprocessor(f: extern "C" fn(Id) -> Id) -> extern "C" fn(Id) -> Id;
}

extern "C" fn say_exception(e: Id) -> Id {
    let name = rust_string(unsafe { send(e, "name") });
    let reason = rust_string(unsafe { send(e, "reason") });
    eprintln!("the system raised {name}: {reason}");
    e
}

/// Says what an exception the system raises is about before it ends the program.
pub fn say_exceptions() {
    unsafe { objc_setExceptionPreprocessor(say_exception) };
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFRelease(obj: Id);
    pub fn CFDataCreate(alloc: Id, bytes: *const u8, len: isize) -> Id;
    pub fn CFURLCreateFromFileSystemRepresentation(alloc: Id, buf: *const u8, len: isize, dir: bool) -> Id;
}

/// A moment in a medium: `value` / `timescale` seconds.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}

impl CMTime {
    pub fn new(value: i64, timescale: i32) -> CMTime {
        CMTime { value, timescale, flags: 1, epoch: 0 }
    }

    /// A time in seconds, in millionths.
    pub fn seconds(s: f64) -> CMTime {
        CMTime::new((s * 1_000_000.0).round() as i64, 1_000_000)
    }

    pub fn is_valid(&self) -> bool {
        self.flags & 1 != 0 && self.timescale != 0
    }

    pub fn as_seconds(&self) -> f64 {
        if self.is_valid() { self.value as f64 / self.timescale as f64 } else { 0.0 }
    }
}

/// A stretch of time: where it starts and how long it lasts.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CMTimeRange {
    pub start: CMTime,
    pub duration: CMTime,
}

pub fn class(name: &str) -> Id {
    let name = CString::new(name).unwrap_or_default();
    unsafe { objc_getClass(name.as_ptr()) }
}

pub fn sel(name: &str) -> Sel {
    let name = CString::new(name).unwrap_or_default();
    unsafe { sel_registerName(name.as_ptr()) }
}

/// `objc_msgSend` as a function of the given type (it must match the method called). On
/// Apple's arm64 the same entry returns structs of any size.
macro_rules! msg {
    ($t:ty) => {
        std::mem::transmute::<*const std::ffi::c_void, $t>($crate::mac::objc_msgSend as *const std::ffi::c_void)
    };
}
pub(crate) use msg;

/// `[obj name]`
pub unsafe fn send(obj: Id, name: &str) -> Id {
    unsafe { msg!(unsafe extern "C" fn(Id, Sel) -> Id)(obj, sel(name)) }
}

/// `[obj name: arg]`
pub unsafe fn send1(obj: Id, name: &str, arg: Id) -> Id {
    unsafe { msg!(unsafe extern "C" fn(Id, Sel, Id) -> Id)(obj, sel(name), arg) }
}

/// `[obj name: a with: b]`
pub unsafe fn send2(obj: Id, name: &str, a: Id, b: Id) -> Id {
    unsafe { msg!(unsafe extern "C" fn(Id, Sel, Id, Id) -> Id)(obj, sel(name), a, b) }
}

/// `[obj name]` for a method giving back a BOOL.
pub unsafe fn yes(obj: Id, name: &str) -> bool {
    unsafe { msg!(unsafe extern "C" fn(Id, Sel) -> bool)(obj, sel(name)) }
}

/// `[obj name]` for a method giving back an integer.
pub unsafe fn integer(obj: Id, name: &str) -> i64 {
    unsafe { msg!(unsafe extern "C" fn(Id, Sel) -> i64)(obj, sel(name)) }
}

/// Whether `obj` answers `name`.
pub unsafe fn answers(obj: Id, name: &str) -> bool {
    !obj.is_null() && unsafe { msg!(unsafe extern "C" fn(Id, Sel, Sel) -> bool)(obj, sel("respondsToSelector:"), sel(name)) }
}

pub fn nsstring(s: &str) -> Id {
    let c = CString::new(s).unwrap_or_default();
    unsafe { msg!(unsafe extern "C" fn(Id, Sel, *const c_char) -> Id)(class("NSString"), sel("stringWithUTF8String:"), c.as_ptr()) }
}

pub fn rust_string(ns: Id) -> String {
    if ns.is_null() {
        return String::new();
    }
    let p = unsafe { msg!(unsafe extern "C" fn(Id, Sel) -> *const c_char)(ns, sel("UTF8String")) };
    if p.is_null() { String::new() } else { unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

pub fn number(v: f64) -> Id {
    unsafe { msg!(unsafe extern "C" fn(Id, Sel, f64) -> Id)(class("NSNumber"), sel("numberWithDouble:"), v) }
}

pub fn int(v: i64) -> Id {
    unsafe { msg!(unsafe extern "C" fn(Id, Sel, i64) -> Id)(class("NSNumber"), sel("numberWithLongLong:"), v) }
}

pub fn boolean(v: bool) -> Id {
    unsafe { msg!(unsafe extern "C" fn(Id, Sel, bool) -> Id)(class("NSNumber"), sel("numberWithBool:"), v) }
}

pub fn dict(pairs: &[(Id, Id)]) -> Id {
    let (keys, values): (Vec<Id>, Vec<Id>) = pairs.iter().copied().unzip();
    unsafe { msg!(unsafe extern "C" fn(Id, Sel, *const Id, *const Id, usize) -> Id)(class("NSDictionary"), sel("dictionaryWithObjects:forKeys:count:"), values.as_ptr(), keys.as_ptr(), pairs.len()) }
}

/// An NSDictionary's value for `key`, as a number.
pub unsafe fn dict_number(d: Id, key: Id) -> Option<f64> {
    if d.is_null() || key.is_null() {
        return None;
    }
    let v = unsafe { send1(d, "objectForKey:", key) };
    if v.is_null() || !unsafe { answers(v, "doubleValue") } {
        return None;
    }
    Some(unsafe { msg!(unsafe extern "C" fn(Id, Sel) -> f64)(v, sel("doubleValue")) })
}

/// An NSArray's items.
pub unsafe fn items(array: Id) -> Vec<Id> {
    if array.is_null() {
        return Vec::new();
    }
    let n = unsafe { msg!(unsafe extern "C" fn(Id, Sel) -> usize)(array, sel("count")) };
    (0..n).map(|i| unsafe { msg!(unsafe extern "C" fn(Id, Sel, usize) -> Id)(array, sel("objectAtIndex:"), i) }).collect()
}

pub fn error_text(err: Id) -> String {
    if err.is_null() { "the system gave no reason".into() } else { rust_string(unsafe { send(err, "localizedDescription") }) }
}

pub fn file_url(path: &str) -> Id {
    unsafe { send1(class("NSURL"), "fileURLWithPath:", nsstring(path)) }
}

/// An autorelease pool, drained when it goes.
pub struct Pool(*mut c_void);

impl Pool {
    pub fn new() -> Pool {
        Pool(unsafe { objc_autoreleasePoolPush() })
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        unsafe { objc_autoreleasePoolPop(self.0) };
    }
}
