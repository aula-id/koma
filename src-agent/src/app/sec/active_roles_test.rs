//! Phase 4: the cyber roles are hidden while the lane is off, and Main never
//! inherits a cyber capability. Both are easy to break by "helpfully" adding a
//! fallback, so both are pinned.

use crate::model::app_config::ModelRole;

#[test]
fn the_cyber_roles_are_hidden_when_security_is_off() {
    let off = ModelRole::visible(false);
    for role in [
        ModelRole::CyberRed,
        ModelRole::CyberBlue,
        ModelRole::CyberResearch,
    ] {
        assert!(!off.contains(&role), "{role:?} is visible while off");
    }
}

#[test]
fn they_appear_once_enabled() {
    let on = ModelRole::visible(true);
    for role in [
        ModelRole::CyberRed,
        ModelRole::CyberBlue,
        ModelRole::CyberResearch,
    ] {
        assert!(on.contains(&role), "{role:?} missing while on");
    }
}

#[test]
fn enabling_only_ever_adds_the_cyber_roles() {
    // Turning security on must not disturb any ordinary role.
    let off = ModelRole::visible(false);
    let on = ModelRole::visible(true);
    for role in &off {
        assert!(on.contains(role), "{role:?} disappeared when enabling");
    }
    assert_eq!(on.len(), off.len() + 3);
}

#[test]
fn main_is_not_a_cyber_role() {
    assert!(!ModelRole::Main.is_cyber());
}

#[test]
fn no_cyber_role_resolves_through_main() {
    // Fail-closed by design: an unassigned cyber role must read as unassigned
    // rather than being quietly served by the coding model.
    let mut config = crate::model::app_config::AppConfig::default();
    config.models.clear();
    let settings = crate::model::settings::Settings::default();
    for role in [
        ModelRole::CyberRed,
        ModelRole::CyberBlue,
        ModelRole::CyberResearch,
    ] {
        assert!(
            crate::app::resolve::resolve_role(&config, &settings, role).is_none(),
            "{role:?} fell back to a model it should not have"
        );
    }
}
