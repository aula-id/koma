use super::*;

#[test]
fn every_inactive_reason_is_distinct_and_non_empty() {
    let all = [
        Inactive::Disabled,
        Inactive::PlanMode,
        Inactive::NoLane,
        Inactive::NoModel,
    ];
    for (i, a) in all.iter().enumerate() {
        assert!(!a.reason().is_empty());
        for b in &all[i + 1..] {
            assert_ne!(a.reason(), b.reason(), "{a:?} vs {b:?}");
        }
    }
}

#[test]
fn a_lane_maps_to_its_role() {
    // The resolution here depends on the enum, not on config, so it is worth
    // pinning: a swapped mapping would gate the wrong capability.
    assert_eq!(Lane::Research.role(), ModelRole::CyberResearch);
    assert_eq!(Lane::Red.role(), ModelRole::CyberRed);
    assert_eq!(Lane::Blue.role(), ModelRole::CyberBlue);
}

use crate::model::app_config::ModelRole;
