//! [`SkillCmdState`] — the working state for the `/skill` hub overlay.
//!
//! A searchable, filterable list of skills loaded from the session's
//! [`SkillRegistry`](crate::model::skill::SkillRegistry). Users can toggle
//! skills on/off from the hub.

use std::collections::BTreeSet;

use crate::model::skill::SkillRegistry;

/// Filter chip displayed above the skill list (radio group).
///
/// Order is left→right in the UI: All → Active → Inactive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillFilterChip {
    /// Show all skills.
    All,
    /// Show only active (loaded) skills.
    Active,
    /// Show only inactive (unloaded) skills.
    Inactive,
}

impl SkillFilterChip {
    /// Cycle right: All → Active → Inactive → All.
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Active,
            Self::Active => Self::Inactive,
            Self::Inactive => Self::All,
        }
    }

    /// Cycle left: All → Inactive → Active → All.
    pub fn prev(self) -> Self {
        match self {
            Self::All => Self::Inactive,
            Self::Active => Self::All,
            Self::Inactive => Self::Active,
        }
    }
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
    ///
    /// Selection sticks to the previously highlighted skill by name when that
    /// skill still appears in the filtered set (typing / chip / toggle).
    /// Otherwise the cursor clamps into the new list.
    pub fn refilter(&mut self) {
        let prev = self.selected_name().map(str::to_owned);
        let q = self.query.to_lowercase();
        self.filtered_idx = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                // Chip filter
                match self.chip {
                    SkillFilterChip::All => {}
                    SkillFilterChip::Active if !e.is_active => return false,
                    SkillFilterChip::Active => {}
                    SkillFilterChip::Inactive if e.is_active => return false,
                    SkillFilterChip::Inactive => {}
                }
                // Query filter
                q.is_empty()
                    || e.name.to_lowercase().contains(&q)
                    || e.description.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();
        if let Some(name) = prev.as_deref() {
            if self.select_by_name(name) {
                return;
            }
        }
        // Clamp
        if self.selected >= self.filtered_idx.len() {
            self.selected = self.filtered_idx.len().saturating_sub(1);
        }
    }

    /// Move the cursor onto the filtered row whose skill name matches `name`.
    /// Returns `true` when found.
    pub fn select_by_name(&mut self, name: &str) -> bool {
        if let Some(fi) = self
            .filtered_idx
            .iter()
            .position(|&ai| self.all.get(ai).is_some_and(|e| e.name == name))
        {
            self.selected = fi;
            true
        } else {
            false
        }
    }

    /// Cycle the filter chip right and refilter (sticky name).
    pub fn chip_next(&mut self) {
        self.chip = self.chip.next();
        self.refilter();
    }

    /// Cycle the filter chip left and refilter (sticky name).
    pub fn chip_prev(&mut self) {
        self.chip = self.chip.prev();
        self.refilter();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, active: bool) -> SkillEntry {
        SkillEntry {
            name: name.into(),
            description: format!("{name} desc"),
            is_active: active,
        }
    }

    fn state(entries: Vec<SkillEntry>) -> SkillCmdState {
        let mut s = SkillCmdState {
            query: String::new(),
            chip: SkillFilterChip::All,
            all: entries,
            filtered_idx: vec![],
            selected: 0,
        };
        s.refilter();
        s
    }

    #[test]
    fn refilter_keeps_selection_by_name_while_typing() {
        let mut s = state(vec![
            entry("alpha", false),
            entry("beta", true),
            entry("gamma", false),
        ]);
        s.selected = 1; // beta
        s.query = "a".into(); // alpha, beta, gamma all match "a"
        s.refilter();
        assert_eq!(s.selected_name(), Some("beta"));

        s.query = "al".into(); // only alpha
        s.refilter();
        assert_eq!(s.selected_name(), Some("alpha"));
    }

    #[test]
    fn refilter_clamps_when_name_leaves_filter() {
        let mut s = state(vec![
            entry("alpha", false),
            entry("beta", true),
            entry("gamma", false),
        ]);
        s.selected = 1; // beta
        s.query = "gamma".into();
        s.refilter();
        assert_eq!(s.selected_name(), Some("gamma"));
        assert_eq!(s.filtered_idx.len(), 1);
    }

    #[test]
    fn set_active_under_active_chip_sticks_or_clamps() {
        let mut s = state(vec![
            entry("alpha", true),
            entry("beta", true),
            entry("gamma", false),
        ]);
        s.chip = SkillFilterChip::Active;
        s.refilter();
        assert_eq!(s.filtered_idx.len(), 2);
        s.selected = 1; // beta
        s.set_active("beta", false); // drops out of Active filter
        assert_eq!(s.selected_name(), Some("alpha"));
        assert_eq!(s.filtered_idx.len(), 1);
    }

    #[test]
    fn chip_cycles_all_active_inactive() {
        assert_eq!(SkillFilterChip::All.next(), SkillFilterChip::Active);
        assert_eq!(SkillFilterChip::Active.next(), SkillFilterChip::Inactive);
        assert_eq!(SkillFilterChip::Inactive.next(), SkillFilterChip::All);
        assert_eq!(SkillFilterChip::All.prev(), SkillFilterChip::Inactive);
        assert_eq!(SkillFilterChip::Inactive.prev(), SkillFilterChip::Active);
        assert_eq!(SkillFilterChip::Active.prev(), SkillFilterChip::All);
    }

    #[test]
    fn inactive_chip_filters_unloaded_only() {
        let mut s = state(vec![
            entry("alpha", false),
            entry("beta", true),
            entry("gamma", false),
        ]);
        s.chip = SkillFilterChip::Inactive;
        s.refilter();
        let names: Vec<_> = s
            .filtered_idx
            .iter()
            .map(|&i| s.all[i].name.as_str())
            .collect();
        assert_eq!(names, vec!["alpha", "gamma"]);
    }
}
