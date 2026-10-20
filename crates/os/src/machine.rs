//! What the computer has to work with, read as work goes on: its cores (on Apple silicon, the
//! fast ones and the frugal ones), its memory and how much of it is free, how much this program
//! holds, whether it is running hot or saving power. Long work (a render) reads these to take
//! what it can without making the computer wait, and runs below what the person is doing.

/// The computer as it is now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Machine {
    /// The fast cores and the frugal ones (all counted fast where there is no telling).
    pub fast: usize,
    pub frugal: usize,
    /// Its memory, and about how much of it is free now (bytes).
    pub memory: u64,
    pub free: u64,
    /// How hot it is running: 0 as usual, 1 warm, 2 hot (the system slows itself), 3 too hot.
    pub heat: u8,
    /// Whether it is saving power (Low Power Mode).
    pub saving: bool,
}

impl Machine {
    /// The cores in all.
    pub fn cores(&self) -> usize {
        (self.fast + self.frugal).max(1)
    }
}

/// The computer as it is now.
pub fn machine() -> Machine {
    #[cfg(target_os = "macos")]
    {
        mac::machine()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        Machine { fast: cores, frugal: 0, memory: 8 << 30, free: 4 << 30, heat: 0, saving: false }
    }
}

/// How much memory this program holds now (bytes; its footprint, as Activity Monitor shows it).
pub fn held() -> u64 {
    #[cfg(target_os = "macos")]
    {
        mac::footprint()
    }
    #[cfg(not(target_os = "macos"))]
    {
        0
    }
}

/// The thread this is called on runs as work the person started and waits for: on the fast
/// cores too, but below what they are doing now (the window, the pointer), which goes first, so
/// the computer stays quick while it works.
pub fn give_way() {
    #[cfg(target_os = "macos")]
    mac::utility();
}

#[cfg(target_os = "macos")]
mod mac {
    use super::Machine;
    use std::ffi::{CString, c_char, c_void};

    unsafe extern "C" {
        fn sysctlbyname(name: *const c_char, old: *mut c_void, len: *mut usize, new: *mut c_void, newlen: usize) -> i32;
        fn task_info(task: u32, flavor: i32, info: *mut u64, count: *mut u32) -> i32;
        fn pthread_set_qos_class_self_np(qos: u32, relative: i32) -> i32;
        static mach_task_self_: u32;
    }

    /// A number the kernel keeps by name (none when it has no such).
    fn number(name: &str) -> Option<u64> {
        let c = CString::new(name).ok()?;
        let mut v = [0u8; 8];
        let mut len = v.len();
        let ok = unsafe { sysctlbyname(c.as_ptr(), v.as_mut_ptr() as *mut c_void, &mut len, std::ptr::null_mut(), 0) } == 0;
        match (ok, len) {
            (true, 4) => Some(u32::from_ne_bytes([v[0], v[1], v[2], v[3]]) as u64),
            (true, 8) => Some(u64::from_ne_bytes(v)),
            _ => None,
        }
    }

    pub fn machine() -> Machine {
        let all = number("hw.ncpu").unwrap_or(1) as usize;
        let fast = number("hw.perflevel0.physicalcpu").map_or(all, |v| v as usize);
        let frugal = number("hw.perflevel1.physicalcpu").map_or(0, |v| v as usize);
        let memory = number("hw.memsize").unwrap_or(8 << 30);
        // how much of it the system counts free, as a share (what Memory Pressure shows)
        let free = number("kern.memorystatus_level").map_or(memory / 2, |pct| memory / 100 * pct.min(100));
        let (heat, saving) = unsafe {
            let info = crate::mac::send(crate::mac::class("NSProcessInfo"), "processInfo");
            let heat = if crate::mac::answers(info, "thermalState") { crate::mac::integer(info, "thermalState").clamp(0, 3) as u8 } else { 0 };
            let saving = crate::mac::answers(info, "isLowPowerModeEnabled") && crate::mac::yes(info, "isLowPowerModeEnabled");
            (heat, saving)
        };
        Machine { fast: fast.max(1), frugal, memory, free, heat, saving }
    }

    /// The process's physical footprint, from the kernel's account of its memory.
    pub fn footprint() -> u64 {
        const TASK_VM_INFO: i32 = 22;
        // (the account is bigger in newer systems: room for it all)
        let mut info = [0u64; 96];
        let mut count = (info.len() * 2) as u32;
        let ok = unsafe { task_info(mach_task_self_, TASK_VM_INFO, info.as_mut_ptr(), &mut count) } == 0;
        // phys_footprint follows the first nineteen fields; its place is 144 bytes in
        if ok && count as usize * 4 >= 152 { info[18] } else { 0 }
    }

    pub fn utility() {
        const QOS_CLASS_USER_INITIATED: u32 = 0x19;
        unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INITIATED, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_computer_is_read() {
        let m = machine();
        assert!(m.cores() >= 1 && m.memory > 0 && m.free <= m.memory && m.heat <= 3);
        #[cfg(target_os = "macos")]
        {
            let before = held();
            let v = vec![1u8; 64 << 20];
            let after = held();
            assert!(before > 0 && after >= before + (32 << 20), "{before} {after}");
            drop(v);
        }
        give_way();
    }
}
