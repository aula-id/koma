use super::*;

struct Archive(std::path::PathBuf);

impl Archive {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("koma-drss-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn append(&self, role: Role, content: &str) -> ChatMessage {
        msglog::append(&self.0, role, content, None, None).unwrap();
        ChatMessage::new(role, content)
    }
}

impl Drop for Archive {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn unavailable_fold_preserves_every_message_newer_than_summary() {
    let archive = Archive::new();
    let mut history = vec![ChatMessage::new(Role::System, "system")];
    history.push(archive.append(Role::User, "old request"));
    history.push(archive.append(Role::Assistant, "old answer"));
    msglog::write_summary(&archive.0, "Old exchange only.", 2, 3).unwrap();
    for i in 0..60 {
        let role = if i % 2 == 0 {
            Role::User
        } else {
            Role::Assistant
        };
        history.push(archive.append(role, &format!("new-message-{i}: {}", "x".repeat(4000))));
    }

    let out = shape(
        history,
        &archive.0,
        &OpenRouterClient::new(),
        &Settings::default(),
        None,
        "continue",
        true,
        118_000,
        false,
        &GoalWire::default(),
    )
    .await;

    assert!(
        out.len() > 60,
        "the summary does not cover any of the 60 new messages"
    );
    for i in 0..60 {
        assert!(out[1..]
            .iter()
            .any(|m| m.content.contains(&format!("new-message-{i}:"))));
    }
    assert_eq!(msglog::read_summary(&archive.0).unwrap().covers_up_to, 2);
    assert_eq!(msglog::max_message_id(&archive.0), 62);
}

#[test]
fn watermark_span_can_exceed_hot_message_limit() {
    let body: Vec<_> = (0..150)
        .map(|_| ChatMessage::new(Role::User, "x".repeat(4000)))
        .collect();
    assert_eq!(hot_keep_n(&body, 150, 40, 8000), 150);
    assert!(hot_keep_n(&body, 1, 40, 8000) < 40);
}
