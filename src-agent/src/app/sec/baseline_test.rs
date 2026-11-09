    use super::*;

    /// The baseline. Regenerate deliberately (and say why in the commit) when the
    /// disabled-mode contract genuinely changes.
    #[test]
    fn disabled_mode_refusals_are_byte_stable() {
        let names = ["sec_http", "sec_ffuf", "sec_nope"];
        let got = render(&refusals(false, &names));
        let want = "\
sec_http -> refused: cyber lane off — enable security mode in /security first
sec_ffuf -> refused: cyber lane off — enable security mode in /security first
sec_nope -> refused: cyber lane off — enable security mode in /security first";
        assert_eq!(got, want, "\n--- got ---\n{got}\n--- want ---\n{want}");
    }

    #[test]
    fn ordinary_tools_are_never_gated() {
        for tool in ordinary_tools() {
            assert!(
                crate::security::sec_refusal(tool, false, |_| false).is_none(),
                "{tool} was gated"
            );
        }
    }

    #[test]
    fn enabling_changes_only_the_sec_names() {
        // The ordinary set stays admitted whatever the lane state.
        let off = refusals(false, &ordinary_tools());
        let on = refusals(true, &ordinary_tools());
        assert_eq!(off, on);
        for (_, got) in on {
            assert!(got.is_none());
        }
    }

    #[test]
    fn a_forged_name_is_refused_in_both_states_but_diagnosed_differently() {
        // The refusal order is lane-off, then advertised-ness, then availability. So
        // OFF reports the lane for a forged name too — the lane is the thing the user
        // turned off — and ON reports the name. Pinned in both states because
        // reordering the checks silently changes the diagnosis without changing the
        // refusal.
        let off = refusals(false, &["sec_nope"]);
        let on = refusals(true, &["sec_nope"]);
        assert!(off[0].1.as_deref().unwrap_or("").contains("lane off"));
        assert!(on[0].1.as_deref().unwrap_or("").contains("not advertised"));
    }

    #[test]
    fn a_lane_off_never_reaches_the_advertised_check() {
        // Off wins, so a real tool and a forged one are indistinguishable while off.
        // That is intended: the lane, not the name, is what the user turned off.
        let rows = refusals(false, &["sec_http", "sec_nope"]);
        assert_eq!(rows[0].1, rows[1].1);
    }

    #[test]
    fn a_signalled_tool_is_never_refused_while_on() {
        // Availability is the LAST check, so with a lane on and a real name the
        // verdict follows the machine's runtime, not the fixture.
        let _ = crate::security::runtime_status_refresh(true);
        let rows = refusals(true, &["sec_http"]);
        assert_eq!(rows[0].1.is_none(), crate::security::cyber_execution_available());
    }

    #[test]
    fn the_fake_advertises_deterministically() {
        // Pin the runtime probe once so a mid-test docker cache flip cannot
        // change the refusal between the two samples.
        let _ = crate::security::runtime_status_refresh(true);
        let a = refusals(true, &["sec_http"]);
        let b = refusals(true, &["sec_http"]);
        assert_eq!(a, b);
    }

    #[test]
    fn lane_state_does_not_leak_into_the_fixture() {
        // `Lane` is per-session; the refusal predicate takes only `enabled`. If a
        // future change makes it lane-aware, this fixture is where that shows up.
        // Force one probe first: back-to-back refusals() used to race the process
        // docker cache (ok=false default → ok=true) on slow linux-arm64 runners.
        //
        // The docker probe expires after a few seconds and can flip under load,
        // so each sample is checked against the availability seen around it:
        // the verdict must follow availability alone, never the lane.
        let _ = crate::security::runtime_status_refresh(true);
        for _lane in [Lane::Research, Lane::Red, Lane::Blue] {
            let (available, got) = sample_with_stable_runtime();
            let want = (!available)
                .then(|| "security execution unavailable: isolated runtime not configured".to_string());
            assert_eq!(got, vec![("sec_http".to_string(), want)]);
        }
    }

    /// Refusals for `sec_http` taken while the runtime probe held still.
    fn sample_with_stable_runtime() -> (bool, Vec<(String, Option<String>)>) {
        for _ in 0..20 {
            let before = crate::security::cyber_execution_available();
            let got = refusals(true, &["sec_http"]);
            if crate::security::cyber_execution_available() == before {
                return (before, got);
            }
        }
        panic!("the runtime probe never held still for one sample");
    }
