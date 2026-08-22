#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::strip_ansi;

#[test]
fn strips_color_codes() {
    // ESC[96m...ESC[0m — the canonical colorized output case
    assert_eq!(strip_ansi("\x1b[96mhello\x1b[0m"), "hello");
}

#[test]
fn plain_text_unchanged() {
    let plain = "just a normal string with no escapes";
    assert_eq!(strip_ansi(plain), plain);
}

#[test]
fn strips_bold_and_multi_param() {
    assert_eq!(strip_ansi("\x1b[1;31merror\x1b[0m: bad"), "error: bad");
}

#[test]
fn strips_mixed_content() {
    assert_eq!(
        strip_ansi("\x1b[32mok\x1b[0m plain \x1b[31mfail\x1b[0m"),
        "ok plain fail"
    );
}
