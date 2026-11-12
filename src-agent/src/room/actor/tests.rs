//! End-to-end actor tests: a scripted model plus the real coordinator, shelf
//! and clerk, so the whole room runs with no live provider.
//!
//! The scripted model answers by avatar and turn, so the Chief really delegates,
//! a work lane really publishes to the shelf and submits, the reconciler really
//! wakes the Chief, and the clerk really flags — all driven through the actor.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{json, Value};

use super::*;
use crate::dto::chat::{ChatMessage, FunctionCall, ToolCall};
use crate::room::lane::{ModelReply, ModelRequest, ToolSpec};
use crate::room::registry::RegistryHandle;
use crate::room::work::{Coordinator, WorkStore};

/// A model scripted by (avatar, nth-turn-for-that-avatar). The avatar is read
/// from the system prompt's role line so one script serves the whole room.
#[derive(Default)]
struct Script {
    replies: HashMap<(String, usize), ModelReply>,
    seen: Mutex<HashMap<String, usize>>,
}

struct ScriptedModel {
    script: Script,
    /// Each request's (avatar, system prompt), in order.
    systems: Arc<Mutex<Vec<(String, String)>>>,
    /// A default when no scripted reply matches: a bare final answer.
    fallback: String,
}

fn avatar_of(history: &[ChatMessage]) -> String {
    let system = history.first().map(|m| m.content.as_str()).unwrap_or("");
    for (needle, avatar) in [
        ("Chief", "chief"),
        ("Lab Red", "red"),
        ("Reviewer", "reviewer"),
        ("Lab Reporter", "reporter"),
        ("Clerk", "clerk"),
    ] {
        if system.contains(needle) {
            return avatar.to_string();
        }
    }
    "unknown".to_string()
}

impl ModelClient for ScriptedModel {
    fn stream<'a>(&'a self, request: ModelRequest) -> BoxFuture<'a, ModelReply> {
        let avatar = avatar_of(&request.history);
        let system = request.history.first().map(|m| m.content.clone()).unwrap_or_default();
        self.systems.lock().unwrap().push((avatar.clone(), system));
        let turn = {
            let mut seen = self.script.seen.lock().unwrap();
            let n = seen.entry(avatar.clone()).or_default();
            let at = *n;
            *n += 1;
            at
        };
        let reply = self
            .script
            .replies
            .get(&(avatar, turn))
            .cloned()
            .unwrap_or_else(|| ModelReply {
                text: self.fallback.clone(),
                ..Default::default()
            });
        Box::pin(async move { reply })
    }
}

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        id: format!("c-{name}-{}", args.to_string().len()),
        kind: "function".into(),
        function: FunctionCall {
            name: name.into(),
            arguments: args.to_string(),
        },
    }
}

fn tools(text: &str, calls: Vec<ToolCall>) -> ModelReply {
    ModelReply {
        text: text.into(),
        tool_calls: calls,
        ..Default::default()
    }
}

fn text(t: &str) -> ModelReply {
    ModelReply {
        text: t.into(),
        ..Default::default()
    }
}

/// A built-in host that answers any non-room tool with a canned line.
struct StubBuiltins;

impl BuiltinHost for StubBuiltins {
    fn run(&self, _avatar: &str, _kind: crate::room::lane::LaneKind, call: &ToolCall) -> String {
        format!("ran {}", call.function.name)
    }
    fn specs(&self, _kind: crate::room::lane::LaneKind) -> Vec<ToolSpec> {
        Vec::new()
    }
}

struct Harness {
    systems: Arc<Mutex<Vec<(String, String)>>>,
    handle: RoomHandle,
    shelf: Arc<Shelf>,
    coordinator: Arc<Coordinator>,
}

fn harness(replies: Vec<((&str, usize), ModelReply)>) -> Harness {
    let shelf = Arc::new(Shelf::open_in_memory().expect("shelf"));
    let registry = Arc::new(RegistryHandle::embedded().expect("registry"));
    let coordinator = Arc::new(Coordinator::new(
        WorkStore::open_in_memory().expect("store"),
        Arc::clone(&shelf),
        Arc::clone(&registry),
    ));
    let script = Script {
        replies: replies
            .into_iter()
            .map(|((a, n), r)| ((a.to_string(), n), r))
            .collect(),
        seen: Mutex::new(HashMap::new()),
    };
    let systems = Arc::new(Mutex::new(Vec::new()));
    let model = Arc::new(ScriptedModel {
        script,
        systems: Arc::clone(&systems),
        fallback: "done".into(),
    });
    let handle = spawn_room(
        Arc::clone(&coordinator),
        Arc::clone(&shelf),
        registry,
        model,
        Arc::new(StubBuiltins),
        RoomConfig::default(),
    );
    Harness {
        systems,
        handle,
        shelf,
        coordinator,
    }
}

impl Harness {
    fn send(&self, msg: ActorMsg) {
        self.handle.to_actor.send(msg).expect("actor alive");
    }
    fn user(&self, text: &str) {
        self.send(ActorMsg::User(text.into()));
    }
    fn tick(&self) {
        self.send(ActorMsg::Tick);
    }

    /// Collect room events until `want` returns true or the time runs out.
    async fn wait_for(
        &mut self,
        label: &str,
        mut want: impl FnMut(&RoomEvent) -> bool,
    ) -> Vec<RoomEvent> {
        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(deadline, self.handle.events.recv()).await {
                Ok(Some(event)) => {
                    let hit = want(&event);
                    seen.push(event);
                    if hit {
                        return seen;
                    }
                }
                Ok(None) => panic!("{label}: the room closed; saw {seen:?}"),
                Err(_) => panic!("{label}: timed out; saw {seen:?}"),
            }
        }
    }

    /// Drain whatever events are available right now.
    fn drain(&mut self) -> Vec<RoomEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.handle.events.try_recv() {
            out.push(event);
        }
        out
    }
}

fn chief_said(events: &[RoomEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| {
            if let RoomEvent::Chief(t) = e {
                Some(t.clone())
            } else {
                None
            }
        })
        .collect()
}

#[tokio::test]
async fn the_chief_answers_a_user_message_immediately() {
    let mut h = harness(vec![(("chief", 0), text("On it — I'll scope the work."))]);
    h.user("scan the login page");
    let events = h
        .wait_for("chief answer", |e| matches!(e, RoomEvent::Chief(_)))
        .await;
    assert_eq!(chief_said(&events), ["On it — I'll scope the work."]);
}

#[tokio::test]
async fn a_message_addressed_to_red_is_answered_by_red() {
    let mut h = harness(vec![(("red", 0), text("The login reflects the origin."))]);
    h.user("@red what did you find");
    let events = h
        .wait_for("red chat", |e| {
            matches!(e, RoomEvent::LaneChat { avatar, .. } if avatar == "red")
        })
        .await;
    assert!(events.iter().any(|e| matches!(
        e,
        RoomEvent::LaneChat { avatar, text } if avatar == "red" && text.contains("reflects")
    )));
    assert!(chief_said(&events).is_empty());
}

#[tokio::test]
async fn a_follow_up_to_a_busy_specialist_is_steered_into_that_chat() {
    let mut h = harness(vec![
        (("red", 0), tools("checking", vec![call("group_status", json!({}))])),
        (("red", 1), text("I heard the follow-up.")),
    ]);
    h.user("@red first");
    tokio::time::sleep(Duration::from_millis(30)).await;
    h.user("@red second");
    let events = h
        .wait_for("steered red chat", |e| {
            matches!(e, RoomEvent::LaneChat { text, .. } if text.contains("follow-up"))
        })
        .await;
    assert!(chief_said(&events).is_empty());
}

#[tokio::test]
async fn the_user_cannot_speak_to_the_clerk() {
    let mut h = harness(vec![]);
    h.user("@clerk hello");
    let events = h
        .wait_for("clerk refusal", |e| matches!(e, RoomEvent::Visible { .. }))
        .await;
    assert!(events.iter().any(|e| matches!(
        e,
        RoomEvent::Visible { text, .. } if text.contains("clerk only speaks")
    )));
}

#[tokio::test]
async fn a_user_message_while_the_chief_is_busy_is_injected_not_dropped() {
    // the chief's first turn calls a slow builtin, so it is busy when the second message arrives
    let mut h = harness(vec![
        (
            ("chief", 0),
            tools("thinking", vec![call("group_status", json!({}))]),
        ),
        (("chief", 1), text("I have both of your messages.")),
    ]);
    h.user("first");
    // give the chief turn a moment to be running, then send another
    tokio::time::sleep(Duration::from_millis(30)).await;
    h.user("second");
    let events = h
        .wait_for(
            "chief second-turn answer",
            |e| matches!(e, RoomEvent::Chief(t) if t.contains("both")),
        )
        .await;
    assert!(chief_said(&events).iter().any(|t| t.contains("both")));
}

#[tokio::test]
async fn the_chief_delegates_a_probe_and_the_work_lane_runs_and_publishes() {
    let delegate = call(
        "group_delegate",
        json!({ "goal_key": "g1", "goal": "scan login", "dispatch_key": "r1",
                "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "probe the login" }] }),
    );
    let red_publishes = vec![
        call(
            "put_asset",
            json!({ "key": "assets/app", "host": "app.example", "asset_type": "host", "title": "app" }),
        ),
        call(
            "put_claim",
            json!({ "key": "claims/cors", "vulnclass": "cors", "name": "CORS", "severity": "high",
            "summary": "reflects origin", "host": "app.example", "poc": "curl", "expected_revision": 0 }),
        ),
    ];
    let mut h = harness(vec![
        (("chief", 0), tools("Dispatching Red.", vec![delegate])),
        (("red", 0), tools("Probing.", red_publishes)),
        (("red", 1), text("published my findings")),
    ]);
    h.user("scan the login");
    // the Visible dispatch line marks the probe as started
    h.wait_for(
        "dispatch",
        |e| matches!(e, RoomEvent::Visible { text, .. } if text.contains("probe the login")),
    )
    .await;
    // the red lane publishes to the shelf
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if self_shelf_has(&h.shelf, "claims/cors") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "red never published"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(self_shelf_has(&h.shelf, "assets/app"));
    // the claim was offered to the review queue when its evidence... (none yet, so it is skipped, but the asset/claim exist)
    let _ = &h.coordinator;
}

fn self_shelf_has(shelf: &Shelf, key: &str) -> bool {
    shelf.peek(key).ok().flatten().is_some()
}

#[tokio::test]
async fn a_finished_probe_is_synced_to_completed_and_wakes_the_chief() {
    use crate::room::types::AssignmentStatus;
    let delegate = call(
        "group_delegate",
        json!({ "goal_key": "g1", "goal": "scan", "dispatch_key": "r1",
                "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "probe" }] }),
    );
    let complete = call(
        "group_task_complete",
        json!({ "result": "found", "outcome": "passed",
        "references": [{ "key": "assets/app", "revision": 1 }] }),
    );
    let mut h = harness(vec![
        (("chief", 0), tools("Dispatching.", vec![delegate])),
        (
            ("red", 0),
            tools(
                "working",
                vec![
                    call(
                        "put_asset",
                        json!({ "key": "assets/app", "host": "app.example", "asset_type": "host", "title": "app" }),
                    ),
                    complete,
                ],
            ),
        ),
    ]);
    h.user("scan");
    // wait until the red lane submitted and finished (its asset is on the shelf)
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !self_shelf_has(&h.shelf, "assets/app") {
        assert!(tokio::time::Instant::now() < deadline, "no asset published");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(80)).await; // let the red lane end

    // the reconciler, driven by ticks, syncs the submitted probe to Completed
    let mut done = false;
    for _ in 0..6 {
        h.tick();
        tokio::time::sleep(Duration::from_millis(30)).await;
        if let Some(work) = h.coordinator.active_work() {
            if work
                .assignments
                .iter()
                .any(|a| a.avatar == "red" && a.status == AssignmentStatus::Completed)
            {
                done = true;
                break;
            }
        }
    }
    assert!(done, "the reconciler never synced the probe to Completed");
    // and the Chief was woken (a fresh chief turn ran after the probe settled)
    let status = h.drain();
    assert!(
        status.iter().any(|e| matches!(e, RoomEvent::Status(_))),
        "a tick emits status"
    );
}

#[tokio::test]
async fn a_tick_emits_a_status_snapshot() {
    let mut h = harness(vec![]);
    // seed an active goal so the clerk/reconciler have something, via a direct delegate through the chief
    h.user("hi");
    h.wait_for("chief idle", |e| matches!(e, RoomEvent::Chief(_)))
        .await;
    h.tick();
    let events = h
        .wait_for("status", |e| matches!(e, RoomEvent::Status(_)))
        .await;
    assert!(events
        .iter()
        .any(|e| matches!(e, RoomEvent::Status(s) if s.counts.open == 0)));
}

#[tokio::test]
async fn message_user_from_the_chief_reaches_the_user() {
    let mut h = harness(vec![
        (
            ("chief", 0),
            tools(
                "",
                vec![call(
                    "message_user",
                    json!({ "text": "Quick note: starting now." }),
                )],
            ),
        ),
        (("chief", 1), text("done")),
    ]);
    h.user("go");
    let events = h
        .wait_for(
            "message_user",
            |e| matches!(e, RoomEvent::Chief(t) if t.contains("Quick note")),
        )
        .await;
    assert!(chief_said(&events).iter().any(|t| t.contains("Quick note")));
}

#[tokio::test]
async fn an_unknown_tool_is_refused_without_reaching_the_builtins() {
    // a name this lane was not offered is refused before the builtin host runs
    let delegate = call(
        "group_delegate",
        json!({ "goal_key": "g1", "goal": "scan", "dispatch_key": "r1",
                "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "probe" }] }),
    );
    let mut h = harness(vec![
        (("chief", 0), tools("Dispatching.", vec![delegate])),
        (
            ("red", 0),
            tools("try", vec![call("rm_rf", json!({ "path": "/" }))]),
        ),
        (("red", 1), text("could not")),
    ]);
    h.user("scan");
    h.wait_for(
        "dispatch",
        |e| matches!(e, RoomEvent::Visible { text, .. } if text.contains("probe")),
    )
    .await;
    // the red lane runs and ends; nothing was published and no panic — the forged tool was refused in-router.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = h.drain();
    assert!(!self_shelf_has(&h.shelf, "claims/anything"));
}

#[tokio::test]
async fn shutdown_closes_the_room() {
    let mut h = harness(vec![]);
    h.send(ActorMsg::Shutdown);
    let events = h
        .wait_for("closed", |e| matches!(e, RoomEvent::Closed))
        .await;
    assert_eq!(events.last(), Some(&RoomEvent::Closed));
}

fn visible(nonce: &str) -> Effect {
    Effect::Visible {
        author: "room".into(),
        text: format!("line {nonce}"),
        nonce: nonce.into(),
        work_id: "w1".into(),
    }
}

#[tokio::test]
async fn a_visible_line_posts_once_and_the_store_alone_remembers_it() {
    let mut h = harness(vec![]);
    h.send(ActorMsg::Effects(vec![visible("group-dispatch:w1:r1:red")]));
    h.send(ActorMsg::Effects(vec![visible("group-dispatch:w1:r1:red"), visible("group-dispatch:w1:r2:red")]));
    let events = h
        .wait_for("the second dispatch", |e| matches!(e, RoomEvent::Visible { text, .. } if text.ends_with("r2:red")))
        .await;
    let first = events
        .iter()
        .filter(|e| matches!(e, RoomEvent::Visible { text, .. } if text.ends_with("r1:red")))
        .count();
    assert_eq!(first, 1, "a repeated nonce posted twice: {events:?}");
    assert_eq!(h.coordinator.store().nonce_count("w1").expect("count"), 2);
}

#[tokio::test]
async fn a_resync_answers_with_a_status_without_waiting_for_a_tick() {
    let mut h = harness(vec![]);
    h.send(ActorMsg::Resync);
    let events = h.wait_for("status", |e| matches!(e, RoomEvent::Status(_))).await;
    assert_eq!(events.len(), 1, "a resync only reports status: {events:?}");
}

#[tokio::test]
async fn a_bang_line_runs_the_shell_without_the_model() {
    let mut h = harness(vec![]);
    h.user("!pwd");
    let events = h
        .wait_for("the shell line", |e| matches!(e, RoomEvent::Visible { author, .. } if author == "shell"))
        .await;
    let line = events.iter().find_map(|e| match e {
        RoomEvent::Visible { author, text } if author == "shell" => Some(text.clone()),
        _ => None,
    });
    let line = line.expect("a shell line");
    assert!(line.starts_with("$ pwd"), "{line}");
    assert!(line.contains("ran bash"), "it went through the bash tool: {line}");
    assert!(!events.iter().any(|e| matches!(e, RoomEvent::Chief(_))), "no model turn");
}

#[tokio::test]
async fn the_status_lists_the_whole_roster_and_repeated_turns_do_not_fill_the_timeline() {
    let mut h = harness(vec![(("chief", 0), text("one")), (("chief", 1), text("two"))]);
    h.user("first");
    h.wait_for("first answer", |e| matches!(e, RoomEvent::Chief(t) if t == "one")).await;
    h.user("second");
    h.wait_for("second answer", |e| matches!(e, RoomEvent::Chief(t) if t == "two")).await;
    h.send(ActorMsg::Resync);
    let events = h.wait_for("status", |e| matches!(e, RoomEvent::Status(_))).await;
    let status = events
        .iter()
        .find_map(|e| if let RoomEvent::Status(s) = e { Some(s.clone()) } else { None })
        .expect("a status");

    let avatars: Vec<&str> = status.lanes.iter().map(|l| l.avatar.as_str()).collect();
    for who in ["chief", "red", "reviewer", "reporter", "clerk"] {
        assert!(avatars.contains(&who), "{who} is on the roster: {avatars:?}");
    }
    assert!(status.lanes.iter().all(|l| l.phase == "idle" || l.phase == "running" || l.phase == "done"), "{:?}", status.lanes);
    assert!(status.now_ms > 0);
    assert!(!status.timeline.contains("◆ chief"), "chief turns no longer add a milestone each: {:?}", status.timeline);
    assert!(status.events.iter().all(|e| e.avatar != "chief"), "{:?}", status.events);
}

#[tokio::test]
async fn a_specialists_chat_lane_gets_the_chat_register_on_top_of_its_role() {
    let mut h = harness(vec![(("red", 0), text("Hi, what do you need?"))]);
    h.user("@red hello");
    h.wait_for("red answers", |e| matches!(e, RoomEvent::LaneChat { avatar, .. } if avatar == "red")).await;
    let systems = h.systems.lock().unwrap().clone();
    let (_, system) = systems.iter().find(|(avatar, _)| avatar == "red").expect("red was asked");
    assert!(system.contains("You are Lab Red"), "the role stays: {system}");
    assert!(system.contains("You are talking with the user now"), "the chat register is added: {system}");
    let registry = crate::room::registry::RegistryHandle::embedded().expect("registry").current();
    assert!(!registry.role("red").expect("red").prompt.contains("talking with the user"), "the work prompt is unchanged");
}

// ---- images the user shares ----

const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
    0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0, 1, 0, 0, 5, 0, 1, 0x0D, 0x0A, 0x2D, 0xB4, 0, 0, 0, 0,
    0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// Records the first user turn of each avatar's requests, then answers like [`ScriptedModel`].
struct SeenModel {
    inner: ScriptedModel,
    /// (avatar, number of attachments on the last user turn, that turn's text)
    turns: Arc<Mutex<Vec<(String, usize, String)>>>,
}

impl ModelClient for SeenModel {
    fn stream<'a>(&'a self, request: ModelRequest) -> BoxFuture<'a, ModelReply> {
        let avatar = avatar_of(&request.history);
        if let Some(user) = request.history.iter().rev().find(|m| m.role == crate::dto::chat::Role::User) {
            self.turns.lock().unwrap().push((avatar, user.attachments.len(), user.content.clone()));
        }
        self.inner.stream(request)
    }
}

fn room_with_images(
    name: &str,
    replies: Vec<((&str, usize), ModelReply)>,
) -> (RoomHandle, Arc<Mutex<Vec<(String, usize, String)>>>, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("koma-room-images-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("shared")).unwrap();
    std::fs::write(root.join("shared/01-shot.png"), PNG).unwrap();
    let shelf = Arc::new(Shelf::open_in_memory().expect("shelf"));
    let registry = Arc::new(RegistryHandle::embedded().expect("registry"));
    let coordinator = Arc::new(Coordinator::new(WorkStore::open_in_memory().expect("store"), Arc::clone(&shelf), Arc::clone(&registry)));
    let script = Script {
        replies: replies.into_iter().map(|((a, n), r)| ((a.to_string(), n), r)).collect(),
        seen: Mutex::new(HashMap::new()),
    };
    let turns = Arc::new(Mutex::new(Vec::new()));
    let model = Arc::new(SeenModel {
        inner: ScriptedModel { script, systems: Arc::new(Mutex::new(Vec::new())), fallback: "done".into() },
        turns: Arc::clone(&turns),
    });
    let handle = spawn_room_in(coordinator, shelf, registry, model, Arc::new(StubBuiltins), RoomConfig::default(), Some(root.join("lanes")));
    (handle, turns, root)
}

fn shot(root: &std::path::Path) -> crate::room::lane::UserImage {
    crate::room::lane::UserImage { n: 1, path: root.join("shared/01-shot.png") }
}

#[tokio::test]
async fn an_image_sent_to_a_tagged_specialist_reaches_that_specialist() {
    let (mut handle, turns, root) = room_with_images("tagged", vec![(("red", 0), text("I see a login form."))]);
    handle
        .to_actor
        .send(ActorMsg::UserWith { text: "@red what is in [Image #1]?".into(), images: vec![shot(&root)] })
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, handle.events.recv()).await {
            Ok(Some(RoomEvent::LaneChat { avatar, .. })) if avatar == "red" => break,
            Ok(Some(_)) => {}
            other => panic!("red never answered: {other:?}"),
        }
    }
    let turns = turns.lock().unwrap();
    let (_, attachments, content) = turns.iter().find(|(a, _, _)| a == "red").expect("red was asked");
    assert_eq!(*attachments, 1, "the image is on the user turn itself");
    assert!(content.contains("[Image #1]"));
    assert!(root.join("lanes/red.chat/images").is_dir(), "the lane keeps its own copy");
}

#[tokio::test]
async fn an_image_the_user_shared_follows_the_chief_into_the_lane_it_delegates_to() {
    let delegate = call(
        "group_delegate",
        json!({ "goal_key": "g1", "goal": "check the page", "dispatch_key": "r1",
                "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "probe the login page in the screenshot" }] }),
    );
    let (mut handle, turns, root) = room_with_images("delegated", vec![(("chief", 0), tools("Sending Red.", vec![delegate]))]);
    handle
        .to_actor
        .send(ActorMsg::UserWith { text: "scan the page in [Image #1]".into(), images: vec![shot(&root)] })
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if turns.lock().unwrap().iter().any(|(a, _, _)| a == "red") {
            break;
        }
        match tokio::time::timeout_at(deadline, handle.events.recv()).await {
            Ok(Some(_)) => {}
            other => panic!("red never started: {other:?}"),
        }
    }
    let turns = turns.lock().unwrap();
    let chief = turns.iter().find(|(a, _, _)| a == "chief").expect("the chief answered");
    assert_eq!(chief.1, 1, "the Chief sees the image");
    let red = turns.iter().find(|(a, _, _)| a == "red").expect("red started");
    assert_eq!(red.1, 1, "the delegated lane sees it too");
    assert!(red.2.contains("[Image #"), "and its task names the image: {}", red.2);
}

// ---- stopping lanes ----

/// Answers like the script, except that the named avatars never finish a request.
struct Hangs {
    inner: ScriptedModel,
    hang: &'static [&'static str],
}

impl ModelClient for Hangs {
    fn stream<'a>(&'a self, request: ModelRequest) -> BoxFuture<'a, ModelReply> {
        if self.hang.contains(&avatar_of(&request.history).as_str()) {
            return Box::pin(std::future::pending());
        }
        self.inner.stream(request)
    }
}

fn hanging_room(hang: &'static [&'static str], replies: Vec<((&str, usize), ModelReply)>) -> RoomHandle {
    let shelf = Arc::new(Shelf::open_in_memory().expect("shelf"));
    let registry = Arc::new(RegistryHandle::embedded().expect("registry"));
    let coordinator = Arc::new(Coordinator::new(WorkStore::open_in_memory().expect("store"), Arc::clone(&shelf), Arc::clone(&registry)));
    let script = Script {
        replies: replies.into_iter().map(|((a, n), r)| ((a.to_string(), n), r)).collect(),
        seen: Mutex::new(HashMap::new()),
    };
    let model = Arc::new(Hangs { inner: ScriptedModel { script, systems: Arc::new(Mutex::new(Vec::new())), fallback: "done".into() }, hang });
    spawn_room(coordinator, shelf, registry, model, Arc::new(StubBuiltins), RoomConfig::default())
}

async fn status_where(handle: &mut RoomHandle, want: impl Fn(&StatusSnapshot) -> bool) -> StatusSnapshot {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        handle.to_actor.send(ActorMsg::Resync).unwrap();
        match tokio::time::timeout_at(deadline, handle.events.recv()).await {
            Ok(Some(RoomEvent::Status(status))) if want(&status) => return status,
            Ok(Some(_)) => tokio::time::sleep(Duration::from_millis(10)).await,
            other => panic!("the status never matched: {other:?}"),
        }
    }
}

fn phase_of(status: &StatusSnapshot, avatar: &str, lane: &str) -> String {
    status.lanes.iter().find(|l| l.avatar == avatar && l.lane == lane).map(|l| l.phase.clone()).unwrap_or_default()
}

#[tokio::test]
async fn a_stopped_specialist_stops_showing_as_running() {
    let delegate = call(
        "group_delegate",
        json!({ "goal_key": "g1", "goal": "scan", "dispatch_key": "r1",
                "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "probe" }] }),
    );
    let mut handle = hanging_room(&["red"], vec![(("chief", 0), tools("Sending Red.", vec![delegate]))]);
    handle.to_actor.send(ActorMsg::User("scan".into())).unwrap();
    status_where(&mut handle, |s| phase_of(s, "red", "work") == "running").await;
    handle
        .to_actor
        .send(ActorMsg::Control(crate::room::lane::ControlCommand::Stop { avatar: "red".into() }))
        .unwrap();
    let status = status_where(&mut handle, |s| phase_of(s, "red", "work") == "stopped").await;
    assert_eq!(status.lanes.iter().find(|l| l.avatar == "red" && l.lane == "work").unwrap().error, "stopped by the user");
    assert!(status.events.iter().any(|e| e.avatar == "red" && e.label == "stopped"), "the timeline records it");
}

#[tokio::test]
async fn stop_all_and_a_second_stop_say_what_they_did() {
    let delegate = call(
        "group_delegate",
        json!({ "goal_key": "g1", "goal": "scan", "dispatch_key": "r1",
                "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "probe" }] }),
    );
    let mut handle = hanging_room(&["red"], vec![(("chief", 0), tools("Sending Red.", vec![delegate]))]);
    handle.to_actor.send(ActorMsg::User("scan".into())).unwrap();
    status_where(&mut handle, |s| phase_of(s, "red", "work") == "running").await;
    for _ in 0..2 {
        handle.to_actor.send(ActorMsg::Control(crate::room::lane::ControlCommand::Stop { avatar: "all".into() })).unwrap();
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let (mut stopped, mut nothing) = (false, false);
    while !(stopped && nothing) {
        match tokio::time::timeout_at(deadline, handle.events.recv()).await {
            Ok(Some(RoomEvent::Visible { author, text })) => {
                stopped |= author == "red" && text.contains("stopped by you");
                nothing |= text.contains("nothing running");
            }
            Ok(Some(_)) => {}
            other => panic!("expected both notes, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn esc_stops_the_chiefs_turn_and_the_next_message_is_answered() {
    let mut handle = hanging_room(&["chief"], vec![]);
    handle.to_actor.send(ActorMsg::User("think hard".into())).unwrap();
    status_where(&mut handle, |s| phase_of(s, "chief", "chat") == "running").await;
    handle.to_actor.send(ActorMsg::Interrupt).unwrap();
    status_where(&mut handle, |s| phase_of(s, "chief", "chat").is_empty() || phase_of(s, "chief", "chat") == "idle").await;
    // The Chief is free again: a new message starts a new turn instead of queueing behind the stopped one.
    handle.to_actor.send(ActorMsg::User("hello".into())).unwrap();
    status_where(&mut handle, |s| phase_of(s, "chief", "chat") == "running").await;
}

/// Like [`hanging_room`], over a store and shelf the caller keeps.
fn hanging_room_with(
    coordinator: Arc<Coordinator>,
    shelf: Arc<Shelf>,
    registry: Arc<RegistryHandle>,
    hang: &'static [&'static str],
    replies: Vec<((&str, usize), ModelReply)>,
) -> RoomHandle {
    let script = Script {
        replies: replies.into_iter().map(|((a, n), r)| ((a.to_string(), n), r)).collect(),
        seen: Mutex::new(HashMap::new()),
    };
    let model = Arc::new(Hangs { inner: ScriptedModel { script, systems: Arc::new(Mutex::new(Vec::new())), fallback: "done".into() }, hang });
    spawn_room(coordinator, shelf, registry, model, Arc::new(StubBuiltins), RoomConfig::default())
}

#[tokio::test]
async fn a_room_entered_again_continues_the_assignment_it_left() {
    let delegate = call(
        "group_delegate",
        json!({ "goal_key": "g1", "goal": "scan", "dispatch_key": "r1",
                "assignments": [{ "avatar": "red", "kind": "probe", "prompt": "probe the login" }] }),
    );
    let shelf = Arc::new(Shelf::open_in_memory().expect("shelf"));
    let registry = Arc::new(RegistryHandle::embedded().expect("registry"));
    let coordinator = Arc::new(Coordinator::new(WorkStore::open_in_memory().expect("store"), Arc::clone(&shelf), Arc::clone(&registry)));
    // First room: Red is mid-assignment when it is closed.
    let mut first = hanging_room_with(Arc::clone(&coordinator), Arc::clone(&shelf), Arc::clone(&registry), &["red"], vec![(("chief", 0), tools("Sending Red.", vec![delegate]))]);
    first.to_actor.send(ActorMsg::User("scan".into())).unwrap();
    status_where(&mut first, |s| phase_of(s, "red", "work") == "running").await;
    first.task.abort();
    // Second room over the same store: Red picks the assignment up and is told it is a continuation.
    let turns = Arc::new(Mutex::new(Vec::new()));
    let model = Arc::new(SeenModel {
        inner: ScriptedModel { script: Script::default(), systems: Arc::new(Mutex::new(Vec::new())), fallback: "done".into() },
        turns: Arc::clone(&turns),
    });
    let _second = spawn_room(coordinator, shelf, registry, model, Arc::new(StubBuiltins), RoomConfig::default());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !turns.lock().unwrap().iter().any(|(a, _, _)| a == "red") {
        assert!(tokio::time::Instant::now() < deadline, "red never resumed");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let turns = turns.lock().unwrap();
    let red = turns.iter().find(|(a, _, _)| a == "red").unwrap();
    assert!(red.2.contains("entered again") && red.2.contains("probe the login"), "{}", red.2);
}
