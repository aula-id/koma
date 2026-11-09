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
fn unassigned_cyber_roles_resolve_through_main() {
    // The lane carries the capability rules, so an unassigned cyber role uses the
    // Main route rather than blocking every lane.
    let mut config = crate::model::app_config::AppConfig::default();
    config.models.clear();
    let settings = crate::model::settings::Settings::default();
    let main = crate::app::resolve::resolve_role(&config, &settings, ModelRole::Main);
    assert!(main.is_some(), "Main always resolves");
    for role in [
        ModelRole::CyberRed,
        ModelRole::CyberBlue,
        ModelRole::CyberResearch,
    ] {
        let got = crate::app::resolve::resolve_role(&config, &settings, role);
        assert_eq!(
            got.as_ref().map(|r| (&r.model_id, &r.endpoint)),
            main.as_ref().map(|r| (&r.model_id, &r.endpoint)),
            "{role:?} should inherit Main"
        );
    }
}
