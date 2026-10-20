//! The undo history as a tree of whole-project states. Undo goes to the state a step was made
//! from, redo to the one last left; making a new step after undoing starts a branch, and the
//! old future stays, reachable with [`History::goto`]. States share what they did not change
//! (compositions are shared until edited), so keeping many is cheap.

use std::collections::BTreeMap;
use std::sync::Arc;

use veymelo_project::Project;

/// One state, and how it was reached.
#[derive(Clone, Debug)]
struct State {
    label: String,
    project: Arc<Project>,
    parent: Option<u64>,
    /// The child redo goes to (the one last made or left).
    redo: Option<u64>,
    /// Steps with the same key, one after another, are one step (a value dragged).
    merge: Option<String>,
}

/// A state as listed for people and agents.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub label: String,
    pub parent: Option<u64>,
    pub current: bool,
    /// Undo reaches it (it is before the current state).
    pub past: bool,
    /// Redo reaches it.
    pub future: bool,
}

#[derive(Clone, Debug)]
pub struct History {
    states: BTreeMap<u64, State>,
    current: u64,
    next: u64,
    /// How many steps back undo can go, and how many states off the line are kept.
    pub limit: usize,
}

/// What the first state is called.
pub const FIRST: &str = "Opened";

impl History {
    pub fn new(project: Arc<Project>) -> History {
        let mut states = BTreeMap::new();
        states.insert(1, State { label: FIRST.into(), project, parent: None, redo: None, merge: None });
        History { states, current: 1, next: 2, limit: 200 }
    }

    pub fn project(&self) -> &Arc<Project> {
        &self.states[&self.current].project
    }

    /// The current state's id.
    pub fn current(&self) -> u64 {
        self.current
    }

    /// The label of the step that made the current state.
    pub fn label(&self) -> &str {
        &self.states[&self.current].label
    }

    /// Make `project` the current state, reached by a step called `label`. A step with the same
    /// `merge` key as the one just made folds into it.
    pub fn record(&mut self, label: &str, project: Arc<Project>, merge: Option<&str>) {
        let here = &self.states[&self.current];
        let folds = merge.is_some() && here.merge.as_deref() == merge && here.parent.is_some() && !self.has_children(self.current);
        if folds {
            let s = self.states.get_mut(&self.current).expect("current state");
            s.project = project;
            s.label = label.to_string();
            return;
        }
        let id = self.next;
        self.next += 1;
        self.states.insert(id, State { label: label.to_string(), project, parent: Some(self.current), redo: None, merge: merge.map(str::to_string) });
        self.states.get_mut(&self.current).expect("current state").redo = Some(id);
        self.current = id;
        self.trim();
    }

    fn has_children(&self, id: u64) -> bool {
        self.states.values().any(|s| s.parent == Some(id))
    }

    pub fn can_undo(&self) -> bool {
        self.states[&self.current].parent.is_some()
    }

    pub fn can_redo(&self) -> bool {
        self.states[&self.current].redo.is_some()
    }

    /// Back one step. The label of the step undone, if there was one.
    pub fn undo(&mut self) -> Option<String> {
        let here = &self.states[&self.current];
        let parent = here.parent?;
        let label = here.label.clone();
        self.current = parent;
        Some(label)
    }

    /// Forward one step. The label of the step redone, if there was one.
    pub fn redo(&mut self) -> Option<String> {
        let next = self.states[&self.current].redo?;
        self.current = next;
        Some(self.states[&next].label.clone())
    }

    /// Go to any state; redo from each state before it then leads back to it.
    pub fn goto(&mut self, id: u64) -> bool {
        if !self.states.contains_key(&id) {
            return false;
        }
        let mut child = id;
        while let Some(parent) = self.states[&child].parent {
            self.states.get_mut(&parent).expect("parent state").redo = Some(child);
            child = parent;
        }
        self.current = id;
        true
    }

    /// The states from the first to the current one.
    fn line_back(&self) -> Vec<u64> {
        let mut line = vec![self.current];
        while let Some(p) = self.states[line.last().expect("not empty")].parent {
            line.push(p);
        }
        line.reverse();
        line
    }

    fn line_forward(&self) -> Vec<u64> {
        let mut out = Vec::new();
        let mut at = self.current;
        while let Some(n) = self.states[&at].redo {
            out.push(n);
            at = n;
        }
        out
    }

    /// Every state, oldest first.
    pub fn list(&self) -> Vec<Entry> {
        let back = self.line_back();
        let forward = self.line_forward();
        self.states
            .iter()
            .map(|(id, s)| Entry {
                id: *id,
                label: s.label.clone(),
                parent: s.parent,
                current: *id == self.current,
                past: *id != self.current && back.contains(id),
                future: forward.contains(id),
            })
            .collect()
    }

    /// Keep at most `limit` steps back, and `limit` states off the current line.
    fn trim(&mut self) {
        let limit = self.limit.max(1);
        let mut back = self.line_back();
        while back.len() > limit + 1 {
            // the first state goes; the next on the line becomes the first, and whatever grew
            // from the old first state on other branches goes with it
            let old = back.remove(0);
            let new_root = back[0];
            self.states.remove(&old);
            self.states.get_mut(&new_root).expect("state").parent = None;
            self.drop_orphans();
        }
        let line: Vec<u64> = back.into_iter().chain(self.line_forward()).collect();
        loop {
            let off: Vec<u64> = self.states.keys().copied().filter(|id| !line.contains(id)).collect();
            if off.len() <= limit {
                break;
            }
            // the oldest state that nothing grew from
            let Some(leaf) = off.iter().copied().find(|id| !self.has_children(*id)) else { break };
            self.states.remove(&leaf);
            for s in self.states.values_mut() {
                if s.redo == Some(leaf) {
                    s.redo = None;
                }
            }
        }
    }

    fn drop_orphans(&mut self) {
        loop {
            let orphans: Vec<u64> = self.states.iter().filter(|(_, s)| s.parent.is_some_and(|p| !self.states.contains_key(&p))).map(|(id, _)| *id).collect();
            if orphans.is_empty() {
                break;
            }
            for id in orphans {
                self.states.remove(&id);
            }
        }
        let alive: Vec<u64> = self.states.keys().copied().collect();
        for s in self.states.values_mut() {
            if s.redo.is_some_and(|r| !alive.contains(&r)) {
                s.redo = None;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project told apart by its next id.
    fn p(n: u64) -> Arc<Project> {
        Arc::new(Project { next_id: n, ..Project::new() })
    }

    fn at(h: &History) -> u64 {
        h.project().next_id
    }

    #[test]
    fn undo_and_redo_walk_the_line() {
        let mut h = History::new(p(0));
        assert_eq!(h.undo(), None);
        h.record("one", p(1), None);
        h.record("two", p(2), None);
        assert_eq!(h.undo().as_deref(), Some("two"));
        assert_eq!(at(&h), 1);
        assert_eq!(h.undo().as_deref(), Some("one"));
        assert_eq!(at(&h), 0);
        assert_eq!(h.redo().as_deref(), Some("one"));
        assert_eq!(h.redo().as_deref(), Some("two"));
        assert_eq!(h.redo(), None);
        assert_eq!(at(&h), 2);
    }

    #[test]
    fn a_new_step_after_undo_keeps_the_old_future_as_a_branch() {
        let mut h = History::new(p(0));
        h.record("one", p(1), None);
        h.record("two", p(2), None);
        let two = h.current();
        h.undo();
        h.record("other", p(3), None);
        assert_eq!(at(&h), 3);
        h.undo();
        assert_eq!(h.redo().as_deref(), Some("other"), "redo follows the newest branch");
        assert!(h.goto(two));
        assert_eq!(at(&h), 2);
        h.undo();
        h.undo();
        h.redo();
        assert_eq!(h.redo().as_deref(), Some("two"), "after a jump, redo leads back along it");
        let list = h.list();
        assert_eq!(list.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(), ["Opened", "one", "two", "other"]);
        assert_eq!(list.iter().find(|e| e.label == "other").unwrap().parent, Some(list[1].id));
        assert!(list.iter().find(|e| e.label == "two").unwrap().current);
        assert!(list[1].past && !list[3].past && !list[3].future);
        assert!(!h.goto(99));
    }

    #[test]
    fn steps_with_one_key_are_one_step() {
        let mut h = History::new(p(0));
        h.record("drag", p(1), Some("drag opacity"));
        h.record("drag", p(2), Some("drag opacity"));
        h.record("drag", p(3), Some("drag opacity"));
        assert_eq!(h.len(), 2);
        assert_eq!(at(&h), 3);
        h.record("drag", p(4), Some("drag scale"));
        assert_eq!(h.len(), 3);
        h.undo();
        assert_eq!(at(&h), 3);
        // after undoing, the same key starts a new step instead of changing an old one
        h.undo();
        h.record("drag", p(5), Some("drag opacity"));
        assert_eq!(h.len(), 4);
    }

    #[test]
    fn old_states_go_past_the_limit() {
        let mut h = History::new(p(0));
        h.limit = 3;
        for i in 1..=10 {
            h.record("step", p(i), None);
        }
        let mut back = 0;
        while h.undo().is_some() {
            back += 1;
        }
        assert_eq!(back, 3);
        assert_eq!(at(&h), 7);
        // states off the line are kept up to the limit too
        let mut h = History::new(p(0));
        h.limit = 2;
        for i in 1..=6 {
            h.record("branch", p(i), None);
            h.undo();
        }
        assert_eq!(h.len(), 1 + 1 + 2, "the first state, the one redo leads to, and two more");
        assert_eq!(h.redo().as_deref(), Some("branch"));
        assert_eq!(at(&h), 6, "the newest branch stays");
    }
}
