//! In-process secure-daemon selftest. A temp session, a scripted model, one
//! user line, one Chief reply. It does not touch the user's home or the network.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures_util::future::BoxFuture;

use crate::room::actor::{ActorMsg, NoBuiltins, RoomConfig, RoomEvent};
use crate::room::lane::{ModelClient, ModelReply, ModelRequest};

use super::assemble::RoomSession;

struct Scripted {
    text: String,
}

impl ModelClient for Scripted {
    fn stream<'a>(&'a self, _request: ModelRequest) -> BoxFuture<'a, ModelReply> {
        let text = self.text.clone();
        Box::pin(async move {
            ModelReply {
                text,
                ..Default::default()
            }
        })
    }
}

pub fn run_secure_selftest() -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(answer_once())
}

async fn answer_once() -> Result<(), String> {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "koma-secure-selftest-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let model = Arc::new(Scripted {
        text: "Hello, I am the Chief.".into(),
    });
    let mut session = RoomSession::open(&dir, model, Arc::new(NoBuiltins), RoomConfig::default())?;
    if session
        .room
        .to_actor
        .send(ActorMsg::User("hi".into()))
        .is_err()
    {
        let _ = std::fs::remove_dir_all(&dir);
        return Err("the room was already closed".into());
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let answered = loop {
        match tokio::time::timeout_at(deadline, session.room.events.recv()).await {
            Ok(Some(RoomEvent::Chief(text))) if text.contains("Chief") => break true,
            Ok(Some(_)) => continue,
            _ => break false,
        }
    };
    let _ = session.room.to_actor.send(ActorMsg::Shutdown);
    drop(session);
    let _ = std::fs::remove_dir_all(&dir);
    if answered {
        Ok(())
    } else {
        Err("the room never answered".into())
    }
}
