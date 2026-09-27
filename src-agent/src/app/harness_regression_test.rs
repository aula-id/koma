use super::*;
use crate::model::app_config::{ModelEntry, ProviderConn};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) fn response(content: Value, finish: &str) -> Value {
    json!({"choices": [{"finish_reason": finish, "message": {"role": "assistant", "content": content,
        "reasoning": "ALLOW"}}], "usage": {"prompt_tokens": 12, "completion_tokens": 2000}})
}

pub(crate) async fn fixture(
    replies: Vec<(Value, u64)>,
    distinct: bool,
) -> (AppConfig, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (reply, delay) in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let (offset, length) = loop {
                let mut chunk = [0u8; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..pos]).to_lowercase();
                    let length = header
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse::<usize>()
                        .unwrap();
                    break (pos + 4, length);
                }
            };
            while bytes.len() < offset + length {
                let mut chunk = [0u8; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            requests
                .push(serde_json::from_slice::<Value>(&bytes[offset..offset + length]).unwrap());
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                let body = reply.to_string();
                let wire = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                let _ = socket.write_all(wire.as_bytes()).await;
            });
        }
        requests
    });
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "fixture".into(),
        endpoint,
        api_key: "test".into(),
        ..Default::default()
    });
    for (role, model) in [
        (ModelRole::Safeguard, "primary"),
        (
            ModelRole::Main,
            if distinct { "fallback" } else { "primary" },
        ),
    ] {
        config.models.push(ModelEntry {
            uuid: format!("{role:?}"),
            model_id: model.into(),
            provider_uuid: "fixture".into(),
            roles: vec![role],
            ..Default::default()
        });
    }
    (config, task)
}

#[test]
fn only_complete_final_decisions_are_valid() {
    for invalid in [
        "",
        "ALLOW",
        "reasoning says safe",
        "{\"allow\":",
        "{}",
        "{\"verdict\":\"unknown\"}",
        "{\"allow\":true,\"verdict\":\"block\"}",
    ] {
        assert!(parse_verdict(invalid).is_none(), "{invalid}");
    }
    assert!(
        parse_verdict(r#"{"allow":true,"reason":"ok"}"#)
            .unwrap()
            .allow
    );
    assert!(
        !parse_verdict(r#"{"allow":false,"reason":"denied"}"#)
            .unwrap()
            .allow
    );
}

#[tokio::test]
async fn truncation_retries_same_route_once_with_larger_budget() {
    for content in [Value::Null, json!("{\"allow\":")] {
        let (config, task) = fixture(
            vec![
                (response(content, "length"), 0),
                (response(json!(r#"{"allow":true}"#), "stop"), 0),
            ],
            false,
        )
        .await;
        let verdict = classify(
            &OpenRouterClient::new(),
            &config,
            &Settings::default(),
            vec![],
            false,
        )
        .await;
        assert!(verdict.available && verdict.allow);
        let requests = task.await.unwrap();
        assert_eq!(
            requests
                .iter()
                .map(|r| r["max_tokens"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![2000, 4000]
        );
    }
}

#[tokio::test]
async fn empty_or_malformed_final_falls_back_without_budget_increase() {
    for content in [Value::Null, json!("malformed")] {
        let (config, task) = fixture(
            vec![
                (response(content, "stop"), 0),
                (response(json!(r#"{"allow":true}"#), "stop"), 0),
            ],
            true,
        )
        .await;
        let verdict = classify(
            &OpenRouterClient::new(),
            &config,
            &Settings::default(),
            vec![],
            false,
        )
        .await;
        assert!(verdict.available && verdict.allow);
        assert!(verdict.reason.contains("fallback"));
        let requests = task.await.unwrap();
        assert_eq!(requests[0]["model"], "primary");
        assert_eq!(requests[1]["model"], "fallback");
        assert_eq!(requests[1]["max_tokens"], 2000);
    }
}

#[tokio::test]
async fn identical_route_and_repeated_truncation_stop_at_bound() {
    let (config, task) = fixture(
        vec![
            (response(Value::Null, "length"), 0),
            (response(Value::Null, "length"), 0),
        ],
        false,
    )
    .await;
    let verdict = classify(
        &OpenRouterClient::new(),
        &config,
        &Settings::default(),
        vec![],
        false,
    )
    .await;
    assert!(!verdict.available && !verdict.allow);
    assert!(verdict.reason.contains("empty"));
    assert!(!verdict.reason.contains("fallback"));
    assert_eq!(task.await.unwrap().len(), 2);
}

#[tokio::test]
async fn timeout_reserves_time_for_main_and_both_failures_are_reported() {
    let (config, task) = fixture(
        vec![
            (response(Value::Null, "stop"), 1000),
            (response(json!("bad"), "stop"), 0),
        ],
        true,
    )
    .await;
    let started = std::time::Instant::now();
    let verdict = classify_bounded(
        &OpenRouterClient::new(),
        &config,
        &Settings::default(),
        vec![],
        false,
        std::time::Duration::from_millis(200),
    )
    .await;
    assert!(!verdict.available);
    assert!(verdict.reason.contains("timeout") && verdict.reason.contains("fallback"));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(task.await.unwrap().len(), 2);
}
