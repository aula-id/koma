//! Writes on the room socket: `room.send`, `room.interrupt` and `room.control`.
//!
//! `room.send` keeps the room's own text semantics: plain text goes as written,
//! so a raw `!cmd` or `@name …` behaves exactly as the room defines it; `shell`
//! and `to` only add the prefix. A room that advertises `sdk_user` gets one
//! `sdk_user` frame per send and answers it with an `sdk_ack`. An older room gets
//! text as a plain `user` frame; attachments need `sdk_user`, so they are refused
//! before anything is written.
use super::reader::Want;
use super::{bound_room_mut, object, out_of_range, peer_unsupported, RoomBinding};
use crate::app::runtime::sdk::connection::BUDGET;
use crate::app::runtime::sdk::methods::HandlerFuture;
use crate::app::runtime::sdk::protocol::{self, Attachment, Input, Reason, Request, RpcError};
use crate::app::runtime::sdk::Adapter;
use crate::room::wire::{RoomClient, RoomControl, SdkAttachment};
use serde_json::{json, Map, Value};

/// `room.control` notes are at most this many characters.
const NOTE_CHARS: usize = 8192;
const MAX_INSERTS: usize = 16;

pub fn handle_send(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(send(r.params, adapter))
}

pub fn handle_interrupt(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(interrupt(r.params, adapter))
}

pub fn handle_control(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(control(r.params, adapter))
}

fn conflicting(message: &str) -> RpcError {
    RpcError::reason(Reason::ConflictingParams, message)
}

fn unknown_avatar(avatar: &str) -> RpcError {
    RpcError::reason(Reason::UnknownAvatar, format!("'{avatar}' is not in the room's roster"))
}

fn string_param(map: &Map<String, Value>, key: &str) -> Result<Option<String>, RpcError> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(out_of_range(format!("{key} must be a string"))),
    }
}

/// What one `room.send` writes, after every check.
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub text: String,
    pub attachments: Vec<SdkAttachment>,
}

/// Check `room.send` params against the roster and compose the wire text.
pub fn compose(map: &Map<String, Value>, roster: &[String]) -> Result<Outgoing, RpcError> {
    let text = string_param(map, "text")?.ok_or_else(|| out_of_range("text is required"))?;
    if text.trim().is_empty() || text.len() > protocol::TEXT_BYTES {
        return Err(out_of_range(format!(
            "text must be nonblank and at most {} bytes",
            protocol::TEXT_BYTES
        )));
    }
    let to = string_param(map, "to")?;
    let shell = match map.get("shell") {
        None => false,
        Some(Value::Bool(shell)) => *shell,
        Some(_) => return Err(out_of_range("shell must be a boolean")),
    };
    let attachments: Vec<Attachment> = match map.get("attachments") {
        None => Vec::new(),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|e| out_of_range(format!("attachments: {e}")))?,
    };
    if shell && (to.is_some() || !attachments.is_empty()) {
        return Err(conflicting("shell excludes to and attachments"));
    }
    let wire_text = if shell {
        format!("!{text}")
    } else if let Some(to) = &to {
        if to == "clerk" {
            return Err(
                RpcError::reason(Reason::ClerkNotAddressable, "the clerk cannot be addressed"),
            );
        }
        if !roster.contains(to) {
            return Err(unknown_avatar(to));
        }
        format!("@{to} {text}")
    } else {
        text.clone()
    };
    Ok(Outgoing {
        text: wire_text,
        attachments: attachments
            .into_iter()
            .map(|attachment| match attachment {
                Attachment::ImageFile { path } => SdkAttachment::ImageFile { path },
                Attachment::PastedText { text, .. } => SdkAttachment::PastedText { text },
            })
            .collect(),
    })
}

/// The v1 `submit` preflight for the attachments: count, sizes, and that
/// each image is an absolute path to a regular file. The text was checked by
/// [`compose`].
fn preflight(outgoing: &Outgoing) -> Result<(), RpcError> {
    let input = Input {
        text: "-".into(),
        attachments: outgoing
            .attachments
            .iter()
            .map(|attachment| match attachment {
                SdkAttachment::ImageFile { path } => Attachment::ImageFile { path: path.clone() },
                SdkAttachment::PastedText { text } => Attachment::PastedText {
                    text: text.clone(),
                    name: None,
                },
            })
            .collect(),
    };
    protocol::attachments(input)
        .map(|_| ())
        .map_err(|e| out_of_range(e.message))
}

async fn send(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let room = bound_room_mut(adapter)?;
    let outgoing = compose(&map, room.roster())?;
    if !outgoing.attachments.is_empty() {
        let checked = outgoing.clone();
        tokio::task::spawn_blocking(move || preflight(&checked))
            .await
            .map_err(|e| RpcError::new("internal", e.to_string()))??;
    }
    let room = bound_room_mut(adapter)?;
    let (acked, inserts) = if room.has_feature("sdk_user") {
        sdk_user(room, &outgoing).await?
    } else if !outgoing.attachments.is_empty() {
        return Err(peer_unsupported(
            "this room cannot take attachments; send text only",
        ));
    } else {
        room.write(&RoomClient::User {
            text: outgoing.text.clone(),
        })
        .await?;
        (false, Vec::new())
    };
    // The room keeps the same text for this line: the message and what each
    // attachment became.
    let echo_line_seq = room.core.echo(&crate::room::daemon::stage::with_inserts(
        &outgoing.text,
        &inserts,
    ));
    let value = json!({
        "sent": true,
        "acked": acked,
        "inserts": inserts,
        "echo_line_seq": echo_line_seq,
    });
    Ok(room.scrub(value).await)
}

async fn sdk_user(
    room: &mut RoomBinding,
    outgoing: &Outgoing,
) -> Result<(bool, Vec<String>), RpcError> {
    let id = uuid::Uuid::new_v4().to_string();
    let message = RoomClient::SdkUser {
        id: id.clone(),
        text: outgoing.text.clone(),
        attachments: outgoing.attachments.clone(),
    };
    let ack = room.request(Want::Ack(id), &message, BUDGET).await?;
    let ack = room.scrub(ack).await;
    if ack.get("ok").and_then(Value::as_bool) != Some(true) {
        let error = ack
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("the room refused the message");
        let mut refused = RpcError::reason(Reason::AttachmentRefused, error);
        if let Some(code) = ack.get("code").and_then(Value::as_str) {
            refused = refused.with_detail(json!({"code": code}));
        }
        return Err(refused);
    }
    let inserts = ack
        .get("inserts")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .take(MAX_INSERTS)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok((true, inserts))
}

async fn interrupt(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let _ = object(params)?;
    let room = bound_room_mut(adapter)?;
    room.write(&RoomClient::Interrupt).await?;
    Ok(json!({"sent": true}))
}

/// Check `room.control` params against the roster.
pub fn control_command(
    map: &Map<String, Value>,
    roster: &[String],
) -> Result<RoomControl, RpcError> {
    let action = string_param(map, "action")?.ok_or_else(|| out_of_range("action is required"))?;
    let avatar = string_param(map, "avatar")?.ok_or_else(|| out_of_range("avatar is required"))?;
    let note = string_param(map, "note")?;
    if let Some(note) = &note {
        let chars = note.chars().count();
        if chars == 0 || chars > NOTE_CHARS {
            return Err(out_of_range(format!(
                "note must be 1 to {NOTE_CHARS} characters"
            )));
        }
    }
    match (action.as_str(), &note) {
        ("redirect", None) => {
            return Err(RpcError::reason(Reason::NoteRequired, "redirect needs a note"))
        }
        ("pause" | "resume" | "stop", Some(_)) => {
            return Err(conflicting("note is allowed only with redirect"))
        }
        ("pause" | "resume" | "stop" | "redirect", _) => {}
        _ => {
            return Err(out_of_range(
                "action must be pause, resume, stop or redirect",
            ))
        }
    }
    if avatar == "chief" {
        return Err(
            RpcError::reason(Reason::NoWorkLane, "the chief has no work lane"),
        );
    }
    if !roster.contains(&avatar) {
        return Err(unknown_avatar(&avatar));
    }
    Ok(match (action.as_str(), note) {
        ("pause", _) => RoomControl::Pause { avatar },
        ("resume", _) => RoomControl::Resume { avatar },
        ("stop", _) => RoomControl::Stop { avatar },
        (_, note) => RoomControl::Redirect {
            avatar,
            note: note.unwrap_or_default(),
        },
    })
}

async fn control(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let room = bound_room_mut(adapter)?;
    let command = control_command(&map, room.roster())?;
    room.write(&RoomClient::Control { command }).await?;
    Ok(json!({"sent": true}))
}
