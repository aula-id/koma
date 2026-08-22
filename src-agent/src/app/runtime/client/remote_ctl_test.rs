use super::RemoteSessionShared;

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
