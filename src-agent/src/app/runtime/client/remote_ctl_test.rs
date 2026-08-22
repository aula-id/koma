use super::{RemoteCtx, RemoteSessionShared};
use crate::remote::RemoteTarget;

#[test]
fn new_attempt_rejects_old_generation_and_closes_password_channel() {
    let shared = RemoteSessionShared::new();
    let (first_tx, first_rx) = std::sync::mpsc::channel();
    let (first, first_cancel) = shared.begin(first_tx);
    let (second_tx, _second_rx) = std::sync::mpsc::channel();
    let (second, _) = shared.begin(second_tx);

    assert!(first_cancel.load(std::sync::atomic::Ordering::Acquire));
    assert!(!shared.is_current(first));
    assert!(shared.is_current(second));
    assert!(first_rx.recv().is_err());
}

#[test]
fn finish_clears_password_sender() {
    let shared = RemoteSessionShared::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let (attempt, _) = shared.begin(tx);
    shared.finish(attempt);
    assert!(rx.recv().is_err());
}

#[test]
fn remote_ctx_is_cloneable_and_make_auth_key_mode() {
    let ctx = RemoteCtx {
        host_id: "h1".into(),
        target: RemoteTarget {
            user: "u".into(),
            host: "example.com".into(),
            port: None,
            key: None,
        },
        password: None,
        koma_path: "/home/u/.local/bin/koma".into(),
    };
    let cloned = ctx.clone();
    assert_eq!(cloned.host_label(), "u@example.com");
    assert!(cloned.make_auth().unwrap().is_none());
    assert!(ctx.password().is_none());
}

#[test]
fn remote_ctx_make_auth_password_mode() {
    let ctx = RemoteCtx {
        host_id: "h1".into(),
        target: RemoteTarget {
            user: "u".into(),
            host: "example.com".into(),
            port: Some(2222),
            key: None,
        },
        password: Some("secret".into()),
        koma_path: "/usr/bin/koma".into(),
    };
    let auth = ctx.make_auth().unwrap().expect("password auth");
    assert_eq!(auth.password(), "secret");
}
