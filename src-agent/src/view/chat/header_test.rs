use super::format_sdlc_mode_label;

#[test]
fn format_sdlc_mode_label_includes_and_truncates_branch() {
    assert_eq!(format_sdlc_mode_label("execute", None), "sdlc:execute");
    assert_eq!(
        format_sdlc_mode_label("assess", Some("  feat/rail-gaps  ")),
        "sdlc:assess · feat/rail-gaps"
    );
    assert_eq!(
        format_sdlc_mode_label("integrate", Some("1234567890123456789012345")),
        "sdlc:integrate · 12345678901234567890123…"
    );
}
