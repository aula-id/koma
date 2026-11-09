use super::*;
use std::time::{Duration, Instant};

fn wait_finished() -> ImagePullResult {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(done) = take_finished() {
            return done;
        }
        assert!(Instant::now() < deadline, "pull job never finished");
        std::thread::sleep(Duration::from_millis(10));
    }
}

// One test: the board is process-global, so the steps must run in order.
#[test]
fn background_pull_reports_progress_refuses_overlap_and_keeps_the_outcome() {
    // An unknown composition fails inside the worker without touching docker.
    start("no-such-composition").expect("start");
    let running = progress();
    if let Some(p) = running.as_ref() {
        assert_eq!(p.id, "no-such-composition");
    }
    if busy_id().is_some() {
        let refused = start("minimal").expect_err("second pull must be refused");
        assert!(refused.contains("already pulling"), "{refused}");
    }
    let done = wait_finished();
    assert!(!done.ok);
    assert!(done.message.contains("unknown composition"), "{}", done.message);
    // Finished: idle, outcome taken once, last result kept for the panel.
    assert!(progress().is_none());
    assert!(busy_id().is_none());
    assert!(take_finished().is_none());
    assert_eq!(last_result(), Some(done));
    // A new pull clears the previous outcome.
    start("still-unknown").expect("start again");
    assert!(last_result().is_none());
    let again = wait_finished();
    assert_eq!(again.id, "still-unknown");
}

#[test]
fn one_line_keeps_the_first_line_and_bounds_length() {
    assert_eq!(one_line("\n  boom  \nmore"), "boom");
    let long = "x".repeat(400);
    assert_eq!(one_line(&long).chars().count(), 161);
}
