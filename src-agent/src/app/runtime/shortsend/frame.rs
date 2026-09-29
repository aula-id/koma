//! Wire-only reduction of computer observations. Storage, the chat, and the
//! archive keep every frame. The outgoing request keeps the latest picture
//! and its coordinates.

use crate::dto::chat::{ChatMessage, Role};

const OBSERVATION_PREFIX: &str = "Computer observation ";

/// Keep the newest computer screenshot and coordinate list. Earlier computer
/// observations lose their attachments and coordinate JSON. Earlier computer
/// tool results keep only the observation id, completed count, and error.
pub fn retain_latest_computer_frame(history: &mut [ChatMessage]) {
    let last_observation = history.iter().rposition(is_observation);
    let results: Vec<Option<String>> = history
        .iter()
        .map(|message| {
            if message.role == Role::Tool {
                superseded_result(&message.content)
            } else {
                None
            }
        })
        .collect();
    let last_result = results.iter().rposition(Option::is_some);
    for (index, message) in history.iter_mut().enumerate() {
        if is_observation(message) && Some(index) != last_observation {
            message.content = superseded_observation(&message.content);
            message.attachments.clear();
        } else if let Some(short) = results[index].clone() {
            if Some(index) != last_result {
                message.content = short;
            }
        }
    }
}

fn is_observation(message: &ChatMessage) -> bool {
    message.role == Role::User && message.content.starts_with(OBSERVATION_PREFIX)
}

fn observation_id_in(content: &str) -> Option<String> {
    let start = content.find('{')?;
    let value: serde_json::Value = serde_json::from_str(&content[start..]).ok()?;
    value
        .get("observation_id")
        .and_then(|item| item.as_str())
        .or_else(|| value.get("id").and_then(|item| item.as_str()))
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn superseded_observation(content: &str) -> String {
    let mut line = String::from(
        "Computer observation superseded. Its picture and coordinates are omitted from this request. Act only on the latest computer observation.",
    );
    if let Some(id) = observation_id_in(content) {
        line.push_str(" observation_id=");
        line.push_str(&id);
    }
    line
}

/// A computer tool result carries an observation with an image path and an OCR
/// status. Anything else, including a result already shortened, stays put.
fn superseded_result(content: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    let observation = value.get("observation")?.as_object()?;
    observation.get("image_path")?.as_str()?;
    observation.get("ocr_status")?.as_str()?;
    let observation_id = value
        .get("observation_id")
        .and_then(|item| item.as_str())
        .or_else(|| observation.get("id").and_then(|item| item.as_str()))
        .unwrap_or("");
    let completed = value
        .get("completed")
        .and_then(|item| item.as_u64())
        .unwrap_or(0);
    let error = value
        .get("error")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    Some(
        serde_json::json!({
            "observation_id": observation_id,
            "completed": completed,
            "error": error,
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::chat::Attachment;

    fn image(n: usize) -> Attachment {
        Attachment {
            kind: Default::default(),
            marker_n: n,
            rel_path: format!("images/{n}.png"),
            mime: "image/png".into(),
        }
    }

    fn observation(n: usize, id: &str) -> ChatMessage {
        ChatMessage::new(
            Role::User,
            format!(
                "Computer observation [Image #{n}]. {{\"observation_id\":\"{id}\",\"text\":[{{\"id\":\"w\",\"x\":{n},\"y\":4}}]}}"
            ),
        )
        .with_attachments(vec![image(n)])
    }

    fn result(id: &str) -> ChatMessage {
        ChatMessage::tool_result(
            id.into(),
            format!(
                "{{\"observation_id\":\"{id}\",\"completed\":1,\"error\":null,\"png\":\"\",\"observation\":{{\"id\":\"{id}\",\"image_path\":\"images/1.png\",\"ocr_status\":\"ok\",\"elements\":[{{\"bounds\":{{\"x\":3,\"y\":4}}}}]}}}}"
            ),
        )
    }

    #[test]
    fn latest_frame_keeps_its_picture_and_a_later_user_message_stays() {
        let mut history = vec![
            observation(1, "old"),
            observation(2, "live"),
            ChatMessage::new(Role::User, "thanks"),
        ];
        retain_latest_computer_frame(&mut history);
        assert!(history[0].attachments.is_empty());
        assert!(!history[0].content.contains("\"x\""));
        assert!(history[0].content.contains("observation_id=old"));
        assert_eq!(history[1].attachments.len(), 1);
        assert!(history[1].content.contains("\"x\":2"));
        assert_eq!(history[2].content, "thanks");
    }

    #[test]
    fn older_computer_results_drop_bounds_and_other_tools_stay() {
        let mut history = vec![
            result("old"),
            ChatMessage::tool_result("other".into(), "{\"ok\":true}".into()),
            result("live"),
        ];
        retain_latest_computer_frame(&mut history);
        assert!(!history[0].content.contains("bounds"));
        assert!(history[0].content.contains("\"observation_id\":\"old\""));
        assert!(history[0].content.contains("\"completed\":1"));
        assert_eq!(history[1].content, "{\"ok\":true}");
        assert!(history[2].content.contains("bounds"));
    }
}
