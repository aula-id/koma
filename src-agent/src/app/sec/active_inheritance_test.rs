//! A sub-agent clones its parent's settings, so `cyber_lane` comes along for
//! free. But `build_tool_ctx` resolves the lane against the CHILD session, so
//! inheritance is only correct while the child carries the same selection and
//! the same role assignment. Pinned here because a silent divergence would give
//! a sub-agent a wider capability than its parent.

use crate::app::sec::lane::Lane;
use crate::model::settings::Settings;

#[test]
fn a_cloned_settings_carries_the_lane() {
    let parent = Settings {
        cyber_lane: Some(Lane::Red),
        ..Default::default()
    };
    let child = parent.clone();
    assert_eq!(child.cyber_lane, Some(Lane::Red));
}

#[test]
fn an_unset_parent_stays_unset_in_the_child() {
    let parent = Settings::default();
    assert_eq!(parent.clone().cyber_lane, None);
}

#[test]
fn the_lane_survives_a_settings_round_trip() {
    // It is persisted per session, so a restart must not silently drop it and
    // leave the session ungated.
    let s = Settings {
        cyber_lane: Some(Lane::Blue),
        ..Default::default()
    };
    let json = serde_json::to_string(&s).expect("serialise");
    let back: Settings = serde_json::from_str(&json).expect("deserialise");
    assert_eq!(back.cyber_lane, Some(Lane::Blue));
}

#[test]
fn an_older_settings_file_without_the_key_still_loads() {
    // `#[serde(default)]` is what keeps every pre-existing settings.json valid.
    let json = serde_json::to_string(&Settings::default()).expect("serialise");
    let mut value: serde_json::Value = serde_json::from_str(&json).expect("parse");
    value.as_object_mut().expect("object").remove("cyber_lane");
    let back: Settings = serde_json::from_value(value).expect("load");
    assert_eq!(back.cyber_lane, None);
}
