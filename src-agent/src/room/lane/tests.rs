use std::collections::BTreeSet;

use super::control::{Action, Command, ControlState};
use super::digest::{Digest, Phase};
use super::event::LaneEvent;
use super::inbox::{Inbox, Item, Priority};

fn running(avatars: &[&str]) -> BTreeSet<String> {
    avatars.iter().map(|s| s.to_string()).collect()
}

// ---- inbox ----

#[test]
fn a_turn_leads_with_user_messages_then_follows_priority() {
    let mut inbox = Inbox::new();
    inbox.push(Item::CheckIn { work_id: "w".into() });
    inbox.push(Item::Wake { work_id: "w".into() });
    inbox.push(Item::User("what's happening?".into()));
    inbox.push(Item::ClerkFlag { avatar: "red".into(), severity: "warn".into(), note: "off scope".into() });
    inbox.push(Item::LaneResult { avatar: "reviewer".into(), summary: "confirmed".into() });
    let order: Vec<Priority> = inbox.drain().iter().map(Item::priority).collect();
    assert_eq!(
        order,
        [Priority::User, Priority::ClerkFlag, Priority::LaneResult, Priority::Wake, Priority::CheckIn]
    );
    assert!(!inbox.has_pending(), "drain clears the inbox");
}

#[test]
fn a_coincident_wake_rides_in_the_same_turn_as_the_user_message() {
    let mut inbox = Inbox::new();
    inbox.push(Item::Wake { work_id: "w".into() });
    inbox.push(Item::User("hi".into()));
    let items = inbox.drain();
    assert_eq!(items.len(), 2, "both delivered in one turn, not two");
    assert_eq!(items[0], Item::User("hi".into()), "the user leads");
}

#[test]
fn ties_within_a_priority_keep_arrival_order() {
    let mut inbox = Inbox::new();
    inbox.push(Item::User("first".into()));
    inbox.push(Item::User("second".into()));
    inbox.push(Item::User("third".into()));
    assert_eq!(
        inbox.drain(),
        [Item::User("first".into()), Item::User("second".into()), Item::User("third".into())]
    );
}

#[test]
fn repeated_wakes_and_check_ins_for_one_work_coalesce() {
    let mut inbox = Inbox::new();
    inbox.push(Item::Wake { work_id: "w1".into() });
    inbox.push(Item::Wake { work_id: "w1".into() });
    inbox.push(Item::CheckIn { work_id: "w1".into() });
    inbox.push(Item::CheckIn { work_id: "w1".into() });
    inbox.push(Item::Wake { work_id: "w2".into() });
    let items = inbox.drain();
    // one wake for w1, one check-in for w1, one wake for w2
    assert_eq!(items.len(), 3);
    assert_eq!(items.iter().filter(|i| matches!(i, Item::Wake { .. })).count(), 2);
    assert_eq!(items.iter().filter(|i| matches!(i, Item::CheckIn { .. })).count(), 1);
}

#[test]
fn has_user_detects_an_unread_message_for_preemption() {
    let mut inbox = Inbox::new();
    inbox.push(Item::Wake { work_id: "w".into() });
    assert!(!inbox.has_user());
    inbox.push(Item::User("urgent".into()));
    assert!(inbox.has_user());
    inbox.drain();
    assert!(!inbox.has_user());
}

// ---- control ----

#[test]
fn pause_then_resume_a_running_lane() {
    let mut ctrl = ControlState::default();
    let run = running(&["red", "reviewer"]);
    assert_eq!(ctrl.apply(Command::Pause { avatar: "red".into() }, &run), Action::Pause("red".into()));
    assert!(ctrl.is_paused("red"));
    assert_eq!(ctrl.paused().cloned().collect::<Vec<_>>(), ["red"]);
    // pausing twice is a no-op
    assert!(matches!(ctrl.apply(Command::Pause { avatar: "red".into() }, &run), Action::Noop(_)));
    assert_eq!(ctrl.apply(Command::Resume { avatar: "red".into() }, &run), Action::Resume("red".into()));
    assert!(!ctrl.is_paused("red"));
}

#[test]
fn stop_aborts_and_clears_any_pause() {
    let mut ctrl = ControlState::default();
    let run = running(&["red"]);
    ctrl.apply(Command::Pause { avatar: "red".into() }, &run);
    assert_eq!(ctrl.apply(Command::Stop { avatar: "red".into() }, &run), Action::Stop("red".into()));
    assert!(!ctrl.is_paused("red"), "stopping clears the pause flag");
}

#[test]
fn redirect_injects_steering_text() {
    let mut ctrl = ControlState::default();
    let run = running(&["red"]);
    let action = ctrl.apply(Command::Redirect { avatar: "red".into(), note: "focus on the API".into() }, &run);
    match action {
        Action::Inject { avatar, text } => {
            assert_eq!(avatar, "red");
            assert!(text.contains("Redirect from the Chief") && text.contains("focus on the API"), "{text}");
        }
        other => panic!("expected an inject, got {other:?}"),
    }
}

#[test]
fn commands_for_an_idle_avatar_are_no_ops_with_a_reason() {
    let mut ctrl = ControlState::default();
    let run = running(&["red"]);
    for cmd in [
        Command::Pause { avatar: "reviewer".into() },
        Command::Stop { avatar: "reviewer".into() },
        Command::Redirect { avatar: "reviewer".into(), note: "x".into() },
        Command::Resume { avatar: "red".into() },
    ] {
        assert!(matches!(ctrl.apply(cmd, &run), Action::Noop(_)));
    }
}

// ---- digest ----

#[test]
fn folds_lane_events_into_a_compact_picture() {
    let mut digest = Digest::new();
    assert!(digest.is_empty());
    assert_eq!(digest.render(), "");

    for event in [
        LaneEvent::Step(0),
        LaneEvent::ToolStarted { name: "sec_run".into() },
        LaneEvent::ToolDone { name: "sec_run".into(), ok: true },
        LaneEvent::Published { key: "assets/web".into(), revision: 1 },
        LaneEvent::Published { key: "claims/a".into(), revision: 1 },
        LaneEvent::Published { key: "assets/web".into(), revision: 2 }, // same key again
        LaneEvent::Step(1),
    ] {
        digest.observe("red", &event);
    }
    let red = digest.status("red").expect("red");
    assert_eq!((red.phase, red.steps), (Phase::Running, 2));
    assert_eq!(red.last_tool.as_deref(), Some("sec_run"));
    assert_eq!(red.published, ["assets/web", "claims/a"], "keys deduplicated, first-seen order");

    let text = digest.render();
    assert!(text.contains("red: running (step 2)"), "{text}");
    assert!(text.contains("last: sec_run") && text.contains("published: assets/web, claims/a"), "{text}");
}

#[test]
fn tracks_submitted_done_and_failed_phases() {
    let mut digest = Digest::new();
    digest.observe("reviewer", &LaneEvent::Step(0));
    digest.observe("reviewer", &LaneEvent::Submitted);
    assert_eq!(digest.status("reviewer").expect("r").phase, Phase::Submitted);
    // a lane may publish after submitting and stays submitted
    digest.observe("reviewer", &LaneEvent::Step(1));
    assert_eq!(digest.status("reviewer").expect("r").phase, Phase::Submitted);
    digest.observe("reviewer", &LaneEvent::Finished { report: "done".into() });
    assert_eq!(digest.status("reviewer").expect("r").phase, Phase::Done);

    digest.observe("red", &LaneEvent::Failed { error: "tool error on sec_run\nstack...".into() });
    let red = digest.status("red").expect("red");
    assert_eq!(red.phase, Phase::Failed);
    let text = digest.render();
    assert!(text.contains("red: failed") && text.contains("— tool error on sec_run"), "{text}");
    assert!(!text.contains("stack..."), "only the first line of the error is shown");
}

#[test]
fn a_failure_after_finishing_sticks() {
    let mut digest = Digest::new();
    digest.observe("red", &LaneEvent::Failed { error: "boom".into() });
    digest.observe("red", &LaneEvent::Finished { report: "late".into() });
    assert_eq!(digest.status("red").expect("red").phase, Phase::Failed);
}

#[test]
fn lanes_render_in_a_stable_order_and_can_be_forgotten() {
    let mut digest = Digest::new();
    digest.observe("reviewer", &LaneEvent::Step(0));
    digest.observe("red", &LaneEvent::Step(0));
    digest.observe("red-api", &LaneEvent::Step(0));
    let text = digest.render();
    let order: Vec<&str> = text.lines().skip(1).map(|l| l.trim().split(':').next().unwrap_or("")).collect();
    assert_eq!(order, ["red", "red-api", "reviewer"], "key order, deterministic");
    digest.forget("red-api");
    assert!(digest.status("red-api").is_none());
}
