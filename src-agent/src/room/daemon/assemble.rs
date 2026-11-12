//! Assembling a room from a session's on-disk state.
//!
//! This opens the session's `room.sqlite` (the shelf and the work tables share
//! it), loads the registry with the session overlay, and spawns the actor with
//! a ticker. It takes the model and tool seams as arguments, so it stays free
//! of the daemon's `AppState`: the daemon builds the real seams, and tests pass
//! fakes.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use super::super::actor::{spawn_room, BuiltinHost, RoomConfig, RoomHandle};
use super::super::lane::ModelClient;
use super::super::registry::{overlay_dir_for, RegistryHandle};
use super::super::shelf::Shelf;
use super::super::work::{Coordinator, WorkStore};

/// How often the actor is ticked (reconciler + clerk cadence).
pub const TICK_MS: u64 = 1_000;

/// A running room plus the ticker that drives it. Dropping it, or sending
/// [`ActorMsg::Shutdown`](super::super::actor::ActorMsg::Shutdown), ends both.
pub struct RoomSession {
    pub room: RoomHandle,
    ticker: JoinHandle<()>,
}

impl RoomSession {
    /// Open the room for `session_dir` and start it. `model` and `builtins` are
    /// the live seams (or test fakes).
    pub fn open(
        session_dir: &Path,
        model: Arc<dyn ModelClient>,
        builtins: Arc<dyn BuiltinHost>,
        config: RoomConfig,
    ) -> Result<Self, String> {
        let db = super::super::room_db_path(session_dir);
        let shelf = Arc::new(Shelf::open(&db).map_err(|e| e.0)?);
        let store = WorkStore::open(&db)?;
        let registry = Arc::new(RegistryHandle::with_overlay(overlay_dir_for(session_dir))?);
        let coordinator = Arc::new(Coordinator::new(store, Arc::clone(&shelf), Arc::clone(&registry)));
        let room = spawn_room(coordinator, shelf, registry, model, builtins, config);

        let to_actor = room.to_actor.clone();
        let ticker = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(TICK_MS));
            interval.tick().await; // the first tick fires immediately; skip it
            loop {
                interval.tick().await;
                if to_actor.send(super::super::actor::ActorMsg::Tick).is_err() {
                    break; // the actor is gone
                }
            }
        });
        Ok(Self { room, ticker })
    }
}

impl Drop for RoomSession {
    fn drop(&mut self) {
        self.ticker.abort();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime};

    use futures_util::future::BoxFuture;

    use super::*;
    use crate::room::actor::{ActorMsg, NoBuiltins, RoomEvent};
    use crate::room::lane::{ModelClient, ModelReply, ModelRequest};

    /// A model that answers each avatar's first turn, then a bare finish.
    struct OneShot(Mutex<HashMap<String, usize>>, String);

    impl ModelClient for OneShot {
        fn stream<'a>(&'a self, request: ModelRequest) -> BoxFuture<'a, ModelReply> {
            let system = request.history.first().map(|m| m.content.clone()).unwrap_or_default();
            let avatar = if system.contains("Chief") { "chief" } else { "other" };
            let mut seen = self.0.lock().unwrap();
            let n = seen.entry(avatar.to_string()).or_default();
            let first = *n == 0;
            *n += 1;
            let reply = if first && avatar == "chief" {
                ModelReply { text: self.1.clone(), ..Default::default() }
            } else {
                ModelReply { text: "done".into(), ..Default::default() }
            };
            Box::pin(async move { reply })
        }
    }

    fn temp_session(tag: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("koma-room-assemble-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[tokio::test]
    async fn a_room_opens_from_a_session_dir_and_answers_the_user() {
        let dir = temp_session("open");
        let model = Arc::new(OneShot(Mutex::new(HashMap::new()), "Hello, I am the Chief.".into()));
        let mut session = RoomSession::open(&dir, model, Arc::new(NoBuiltins), RoomConfig::default()).expect("open");

        session.room.to_actor.send(ActorMsg::User("hi".into())).expect("send");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let answered = loop {
            match tokio::time::timeout_at(deadline, session.room.events.recv()).await {
                Ok(Some(RoomEvent::Chief(text))) if text.contains("Chief") => break true,
                Ok(Some(_)) => continue,
                _ => break false,
            }
        };
        assert!(answered, "the room never answered from its on-disk session");

        // the database was created in the session dir and the ticker is running
        assert!(crate::room::room_db_path(&dir).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn opening_a_room_whose_db_cannot_be_created_fails() {
        // a path under a file (not a directory) cannot hold room.sqlite
        let dir = temp_session("bad");
        let file = dir.join("not-a-dir");
        std::fs::write(&file, b"x").expect("write");
        let model = Arc::new(OneShot(Mutex::new(HashMap::new()), "x".into()));
        let result = RoomSession::open(&file, model, Arc::new(NoBuiltins), RoomConfig::default());
        assert!(result.is_err(), "opening under a file should fail");
        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn close(mut session: RoomSession) {
        session.ticker.abort();
        let _ = session.room.to_actor.send(ActorMsg::Shutdown);
        let task = std::mem::replace(&mut session.room.task, tokio::spawn(async {}));
        let _ = task.await;
    }

    #[tokio::test]
    async fn a_visible_nonce_is_not_posted_again_after_reopen() {
        let dir = temp_session("nonce");
        let model = Arc::new(OneShot(Mutex::new(HashMap::new()), "idle".into()));
        let mut session = RoomSession::open(&dir, model, Arc::new(NoBuiltins), RoomConfig::default()).expect("open");
        let effect = crate::room::work::Effect::Visible {
            author: "chief".into(),
            text: "Assignment for Red: probe".into(),
            nonce: "group-dispatch:w1:r1:red".into(),
        };
        session.room.to_actor.send(ActorMsg::Effects(vec![effect.clone()])).expect("send");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let posted = loop {
            match tokio::time::timeout_at(deadline, session.room.events.recv()).await {
                Ok(Some(RoomEvent::Visible { text, .. })) if text.contains("probe") => break true,
                Ok(Some(_)) => continue,
                _ => break false,
            }
        };
        assert!(posted, "the first room never posted the dispatch");
        close(session).await;

        let model = Arc::new(OneShot(Mutex::new(HashMap::new()), "idle".into()));
        let mut session = RoomSession::open(&dir, model, Arc::new(NoBuiltins), RoomConfig::default()).expect("reopen");
        session.room.to_actor.send(ActorMsg::Effects(vec![effect])).expect("send");
        let deadline = tokio::time::Instant::now() + Duration::from_millis(400);
        let repeated = loop {
            match tokio::time::timeout_at(deadline, session.room.events.recv()).await {
                Ok(Some(RoomEvent::Visible { text, .. })) if text.contains("probe") => break true,
                Ok(Some(_)) => continue,
                _ => break false,
            }
        };
        assert!(!repeated, "reopening posted the same dispatch again");
        close(session).await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
