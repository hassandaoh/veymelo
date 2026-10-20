//! Simulations run step by step from the layer's start: their states kept every so often, so
//! any frame is reached from the nearest kept state before it (scrubbing back and forth costs
//! little), and a frame rendered on its own equals the same frame reached by playing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Steps a second.
pub const RATE: f64 = 60.0;

/// The step reached at layer time `t` (seconds).
pub fn step_at(t: f64) -> u64 {
    if t <= 0.0 { 0 } else { (t * RATE + 1e-6).floor() as u64 }
}

/// How often a state is kept, in steps.
const EVERY: u64 = 30;
/// How many simulations are kept (the least lately used goes first).
const KEPT: usize = 6;
/// How many states one simulation keeps (past them, every other is let go).
const MOST: usize = 64;

/// A simulation's kept states, by step.
struct Run<S> {
    states: Vec<(u64, Arc<S>)>,
    used: u64,
}

/// Kept simulations by key, and a clock of their use.
type Runs<S> = Option<(HashMap<u64, Run<S>>, u64)>;

/// Kept simulations of one kind, by key.
pub struct Stepped<S> {
    runs: Mutex<Runs<S>>,
}

impl<S: Clone> Stepped<S> {
    pub const fn new() -> Stepped<S> {
        Stepped { runs: Mutex::new(None) }
    }

    /// The state after `steps` steps of simulation `key`: from `start`, each step by
    /// `step(state, n)` (n from 0).
    pub fn run(&self, key: u64, steps: u64, start: impl FnOnce() -> S, step: impl Fn(&mut S, u64)) -> Arc<S> {
        // the nearest kept state at or before the step
        let found = {
            let mut g = self.runs.lock().unwrap_or_else(|e| e.into_inner());
            let (runs, clock) = g.get_or_insert_with(|| (HashMap::new(), 0));
            *clock += 1;
            let now = *clock;
            runs.get_mut(&key).and_then(|r| {
                r.used = now;
                r.states.iter().rev().find(|(n, _)| *n <= steps).cloned()
            })
        };
        if let Some((n, s)) = &found
            && *n == steps
        {
            return s.clone();
        }
        let (mut at, mut state) = match found {
            Some((n, s)) => (n, (*s).clone()),
            None => (0, start()),
        };
        let mut keep = Vec::new();
        if at == 0 {
            keep.push((0, Arc::new(state.clone())));
        }
        while at < steps {
            step(&mut state, at);
            at += 1;
            if at % EVERY == 0 {
                keep.push((at, Arc::new(state.clone())));
            }
        }
        let out = Arc::new(state);
        let mut g = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        let (runs, clock) = g.get_or_insert_with(|| (HashMap::new(), 0));
        if !runs.contains_key(&key) && runs.len() >= KEPT
            && let Some(old) = runs.iter().min_by_key(|(_, r)| r.used).map(|(k, _)| *k)
        {
            runs.remove(&old);
        }
        let r = runs.entry(key).or_insert(Run { states: Vec::new(), used: *clock });
        for (n, s) in keep {
            if let Err(i) = r.states.binary_search_by_key(&n, |(m, _)| *m) {
                r.states.insert(i, (n, s));
            }
        }
        while r.states.len() > MOST {
            // thin out: every other kept state but the first
            let mut k = 0;
            r.states.retain(|_| {
                k += 1;
                k == 1 || k % 2 == 0
            });
        }
        out
    }
}

/// A number standing for everything an effect's simulation depends on: its settings (keys and
/// all) and the layer's seed.
pub fn key_of(ctx: &crate::Ctx, salt: u64) -> u64 {
    let text = format!("{:?}", ctx.effect);
    // FNV-1a over the settings as written
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    crate::noise::mix(&[h, ctx.seed, salt])
}
