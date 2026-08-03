//! [`SkillCmdState`] — the working state for the `/skill` hub overlay.
//!
//! A searchable, filterable list of skills loaded from the session's
//! [`SkillRegistry`](crate::model::skill::SkillRegistry). Users can toggle
//! skills on/off from the hub.

use std::collections::BTreeSet;

use crate::model::skill::SkillRegistry;

/// Filter chip displayed above the skill list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillFilterChip {
    /// Show all skills.
    All,
    /// Show only active (loaded) skills.
    Active,
}

/// A single row in the skill hub.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    /// Skill name.
    pub name: String,
    /// One-line description.
    pub description: String,
    /// Whether this skill is currently loaded into active_skills.
    pub is_active: bool,
}

/// Working state for the `/skill` hub overlay.
///
/// `all` holds every skill from the registry; `filtered_idx` is a subset of
/// indices into `all` that match `query` + the active chip filter.
/// `selected` is an index into `filtered_idx` (not into `all`).
#[derive(Debug, Clone)]
pub struct SkillCmdState {
    /// The user's live search string (updated on every keypress).
    pub query: String,
    /// Active filter chip.
    pub chip: SkillFilterChip,
    /// Every skill entry, unfiltered, in display order.
    pub all: Vec<SkillEntry>,
    /// Indices into `all` of entries that match the current `query` + chip.
    pub filtered_idx: Vec<usize>,
    /// Cursor position within `filtered_idx`.
    pub selected: usize,
}

impl SkillCmdState {
    /// Build the skill hub state from the session's registry + active set.
    pub fn new(registry: &SkillRegistry, active_skills: &BTreeSet<String>) -> Self {
        let all: Vec<SkillEntry> = registry
            .list()
            .into_iter()
            .map(|def| SkillEntry {
                name: def.name.clone(),
                description: def.description.clone(),
                is_active: active_skills.contains(&def.name),
            })
            .collect();
        let mut s = Self {
            query: String::new(),
            chip: SkillFilterChip::All,
            all,
            filtered_idx: vec![],
            selected: 0,
        };
        s.refilter();
        s
    }

    /// Rebuild `filtered_idx` from `all` using the current `query` and `chip`.
    pub fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        self.filtered_idx = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                // Chip filter
                match self.chip {
                    SkillFilterChip::Active if !e.is_active => return false,
                    _ => {}
                }
                // Query filter
                q.is_empty()
                    || e.name.to_lowercase().contains(&q)
                    || e.description.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();
        // Clamp
        if self.selected >= self.filtered_idx.len() {
            self.selected = self.filtered_idx.len().saturating_sub(1);
        }
    }

    /// Move the cursor up one row (clamps at 0).
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move the cursor down one row (clamps at the last filtered entry).
    pub fn move_down(&mut self) {
        if self.selected + 1 < self.filtered_idx.len() {
            self.selected += 1;
        }
    }

    /// Return the name of the currently highlighted skill, or `None` when the
    /// filtered list is empty.
    pub fn selected_name(&self) -> Option<&str> {
        self.filtered_idx
            .get(self.selected)
            .and_then(|&i| self.all.get(i))
            .map(|e| e.name.as_str())
    }

    /// Update the `is_active` flag for a skill and refilter.
    pub fn set_active(&mut self, name: &str, active: bool) {
        if let Some(entry) = self.all.iter_mut().find(|e| e.name == name) {
            entry.is_active = active;
        }
        self.refilter();
    }
}
