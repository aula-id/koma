//! Which skills each lane has loaded, kept in the lane's own directory.
//!
//! A coding session keeps loaded skills in its session tail. A lane has no such tail, so its loaded
//! skills are a short list of names in `<lane dir>/skills.json`. Each run rebuilds the lane's system
//! prompt from that list and the skill files, so a loaded skill is still there on the next run, and an
//! unloaded one is gone from it.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Mutex;

#[derive(Default)]
pub struct SkillBook {
    lanes: Mutex<HashMap<String, BTreeSet<String>>>,
}

fn file(lane_dir: &Path) -> std::path::PathBuf {
    lane_dir.join("skills.json")
}

impl SkillBook {
    fn with<R>(&self, lane: &str, lane_dir: Option<&Path>, f: impl FnOnce(&mut BTreeSet<String>) -> R) -> R {
        let mut lanes = self.lanes.lock().unwrap_or_else(|e| e.into_inner());
        let set = lanes.entry(lane.to_string()).or_insert_with(|| {
            lane_dir
                .and_then(|d| std::fs::read_to_string(file(d)).ok())
                .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
                .map(|v| v.into_iter().collect())
                .unwrap_or_default()
        });
        f(set)
    }

    fn save(&self, lane: &str, lane_dir: Option<&Path>) {
        let Some(dir) = lane_dir else { return };
        let names: Vec<String> = self.with(lane, lane_dir, |s| s.iter().cloned().collect());
        if std::fs::create_dir_all(dir).is_ok() {
            let _ = std::fs::write(file(dir), serde_json::to_string(&names).unwrap_or_default());
        }
    }

    pub fn active(&self, lane: &str, lane_dir: Option<&Path>) -> Vec<String> {
        self.with(lane, lane_dir, |s| s.iter().cloned().collect())
    }

    pub fn load(&self, lane: &str, lane_dir: Option<&Path>, name: &str) {
        self.with(lane, lane_dir, |s| s.insert(name.to_string()));
        self.save(lane, lane_dir);
    }

    pub fn unload(&self, lane: &str, lane_dir: Option<&Path>, name: &str) -> bool {
        let had = self.with(lane, lane_dir, |s| s.remove(name));
        self.save(lane, lane_dir);
        had
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loaded_skill_survives_a_restart_and_an_unloaded_one_does_not() {
        let dir = std::env::temp_dir().join(format!("koma-skillbook-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let book = SkillBook::default();
        book.load("red.work", Some(&dir), "jwt-review");
        book.load("red.work", Some(&dir), "sql-hunt");
        assert_eq!(book.active("red.work", Some(&dir)), ["jwt-review", "sql-hunt"]);
        // A new process: nothing in memory, the file is what remains.
        let again = SkillBook::default();
        assert_eq!(again.active("red.work", Some(&dir)), ["jwt-review", "sql-hunt"]);
        assert!(again.unload("red.work", Some(&dir), "jwt-review"));
        assert!(!again.unload("red.work", Some(&dir), "jwt-review"));
        assert_eq!(SkillBook::default().active("red.work", Some(&dir)), ["sql-hunt"]);
        // Another lane has its own directory and is untouched.
        assert!(SkillBook::default().active("red.chat", Some(&dir.join("chat"))).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
