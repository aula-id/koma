use super::*;

#[test]
fn byte_units_match_install_sh() {
    assert_eq!(format_bytes_whole(0), "0 B");
    assert_eq!(format_bytes_whole(1023), "1023 B");
    assert_eq!(format_bytes_whole(1024), "1 KB");
    assert_eq!(format_bytes_whole(1500), "1 KB");
    assert_eq!(format_bytes_whole(1024 * 1024), "1 MB");
    assert_eq!(format_bytes_whole(1024 * 1024 * 1024), "1 GB");
}

#[test]
fn elapsed_clock_rolls_over() {
    assert_eq!(format_elapsed(Duration::from_secs(0)), "0s");
    assert_eq!(format_elapsed(Duration::from_secs(12)), "12s");
    assert_eq!(format_elapsed(Duration::from_secs(90)), "1m 30s");
    assert_eq!(format_elapsed(Duration::from_secs(3600 + 120)), "1h 02m");
}

#[test]
fn headline_id_is_not_a_layer() {
    let mut meter = PullMeter::new();
    meter.on_event(
        "Pulling from aula-id/koma-safety",
        "0.4.10-minimal",
        None,
        None,
    );
    let view = meter.view(0, Duration::from_secs(1));
    assert_eq!(view.phase, Phase::Contacting);
    assert_eq!(meter.layer_count(), 0);
}

#[test]
fn download_bar_uses_every_layer_that_reported_a_size() {
    let mut meter = PullMeter::new();
    meter.on_event("Pulling fs layer", "aaa", None, None);
    meter.on_event("Pulling fs layer", "bbb", None, None);
    meter.on_event("Downloading", "aaa", Some(50), Some(100));
    meter.on_event("Downloading", "bbb", Some(10), Some(100));
    let mid = meter.view(0, Duration::from_secs(4));
    assert_eq!(mid.phase, Phase::Downloading);
    assert_eq!(mid.current, 60);
    assert_eq!(mid.total, 200);

    meter.on_event("Download complete", "aaa", None, None);
    let done_one = meter.view(0, Duration::from_secs(5));
    assert_eq!(done_one.current, 110);
    assert_eq!(done_one.total, 200);

    meter.on_event("Downloading", "bbb", Some(100), Some(100));
    meter.on_event("Download complete", "bbb", None, None);
    meter.on_event("Extracting", "aaa", Some(20), Some(80));
    let extracting = meter.view(0, Duration::from_secs(8));
    assert_eq!(extracting.phase, Phase::Extracting);
    assert_eq!(extracting.current, 20);
    assert_eq!(extracting.total, 80);

    meter.on_event(
        "Status: Downloaded newer image for ghcr.io/aula-id/koma-safety:0.4.10-minimal",
        "",
        None,
        None,
    );
    assert!(meter.saw_status());
    assert!(!meter.up_to_date());
    assert_eq!(meter.view(0, Duration::ZERO).phase, Phase::Ready);
}

#[test]
fn an_early_extract_does_not_hide_the_download() {
    let mut meter = PullMeter::new();
    meter.on_event("Extracting", "aaa", Some(10), Some(10));
    meter.on_event("Downloading", "bbb", Some(20), Some(200));
    let view = meter.view(0, Duration::from_secs(3));
    assert_eq!(view.phase, Phase::Downloading);
    assert_eq!(view.current, 20);
    assert_eq!(view.total, 200);
}

#[test]
fn cached_layers_do_not_zero_the_bar() {
    let mut meter = PullMeter::new();
    meter.on_event("Download complete", "cached", None, None);
    meter.on_event("Downloading", "fresh", Some(25), Some(100));
    let view = meter.view(0, Duration::from_secs(2));
    assert_eq!(view.phase, Phase::Downloading);
    assert_eq!(view.current, 25);
    assert_eq!(view.total, 100);
}

#[test]
fn up_to_date_status_is_not_a_download() {
    let mut meter = PullMeter::new();
    meter
        .ingest(r#"{"status":"Status: Image is up to date for example:latest"}"#)
        .unwrap();
    assert!(meter.saw_status());
    assert!(meter.up_to_date());
}

#[test]
fn registry_error_is_returned() {
    let mut meter = PullMeter::new();
    let err = meter.ingest(r#"{"error":"manifest unknown"}"#).unwrap_err();
    assert_eq!(err, "manifest unknown");
}

#[test]
fn download_line_names_bytes_and_rate() {
    let view = PullView {
        phase: Phase::Downloading,
        spinner: 0,
        current: 50 * 1024 * 1024,
        total: 100 * 1024 * 1024,
        bps: 2 * 1024 * 1024,
        elapsed: Duration::from_secs(12),
        layers_done: 1,
        layers: 4,
    };
    let line = render_pull_line(&view);
    assert!(line.contains("50.0%"), "{line}");
    assert!(line.contains("50 MB / 100 MB"), "{line}");
    assert!(line.contains("2 MB/s"), "{line}");
    assert!(line.contains("12s"), "{line}");
    assert_eq!(line.chars().filter(|ch| *ch == '#').count(), 16, "{line}");
}

#[test]
fn extracting_line_does_not_pretend_bytes_are_still_moving() {
    let view = PullView {
        phase: Phase::Extracting,
        spinner: 1,
        current: 40,
        total: 80,
        bps: 0,
        elapsed: Duration::from_secs(30),
        layers_done: 2,
        layers: 4,
    };
    let line = render_pull_line(&view);
    assert!(line.contains("extracting"), "{line}");
    assert!(line.contains("50.0%"), "{line}");
    assert!(!line.contains("MB/s"), "{line}");
}

#[test]
fn summaries_distinguish_skip_update_and_download() {
    let skipped = PullReport {
        skipped: true,
        up_to_date: true,
        bytes: 2 * 1024 * 1024,
        elapsed: Duration::ZERO,
    };
    assert_eq!(skipped.summary(), "already local, 2 MB");

    let fresh = PullReport {
        skipped: false,
        up_to_date: false,
        bytes: 180 * 1024 * 1024,
        elapsed: Duration::from_secs(36),
    };
    assert_eq!(fresh.summary(), "180 MB in 36s");

    let same = PullReport {
        skipped: false,
        up_to_date: true,
        bytes: 0,
        elapsed: Duration::from_secs(1),
    };
    assert_eq!(same.summary(), "up to date");
}
