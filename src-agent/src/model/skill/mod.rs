//! Agent Skill discovery and loading: scan known directories for SKILL.md files,
//! build a catalogue, and provide on-demand body loading.
//!
//! ```text
//! Skills are user-authored markdown files with optional YAML frontmatter.
//! The filename/dir stem is the skill name (lowercased, validated).
//!
//! Layout per root:
//!   skills/
//!     foo.md                 ← flat: name = "foo"
//!     bar/SKILL.md           ← dir form: name = "bar"
//!     bar/references/...     ← reachable after load via read/glob
//!
//! Discovery tiers (later overrides earlier by name):
//!   1. ~/.koma/skills/       (global)
//!   2. <workdir>/.claude/skills/  (Claude compat)
//!   3. <workdir>/.agent/skills/   (koma project)
//!   4. <workdir>/.agents/skills/  (koma project)
//!   5. configured extra roots     (External, read-only)
//! ```

mod def;
mod parse;
mod persistence;
mod registry;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

pub use def::{
    SkillCatalogueEntry, SkillDetail, SkillDuplicateInput, SkillIdentityInput, SkillItemOutcome,
    SkillSource,
};
pub(crate) use persistence::update_owned_skill_with_generation;
pub use persistence::{
    create_owned_skill, delete_owned_skill, duplicate_to_owned, export_owned_skill_zip,
    install_owned_skill_zip, update_owned_skill, OwnedSkillTarget, SkillEdit,
};
pub use registry::{valid_extra_skill_roots, SkillRegistry};
