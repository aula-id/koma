#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::append_ext_context;
use std::collections::BTreeMap;

/// Blobs are appended in deterministic BTreeMap key order (alpha before zebra),
/// each as `\n\n# Extension context: <id>\n<text>`, and a blank blob is skipped.
#[test]
fn append_is_ordered_and_skips_blank() {
    let mut ctx = BTreeMap::new();
    ctx.insert("zebra.ext".to_string(), "z-blob".to_string());
    ctx.insert("alpha.ext".to_string(), "a-blob".to_string());
    ctx.insert("blank.ext".to_string(), "   ".to_string());
    let mut dst = String::from("HEAD");
    append_ext_context(&mut dst, &ctx);
    assert_eq!(
        dst,
        "HEAD\n\n# Extension context: alpha.ext\na-blob\n\n# Extension context: zebra.ext\nz-blob"
    );
}

/// An empty map is a no-op — the volatile tail is byte-identical to before.
#[test]
fn empty_map_is_noop() {
    let ctx: BTreeMap<String, String> = BTreeMap::new();
    let mut dst = String::from("HEAD");
    append_ext_context(&mut dst, &ctx);
    assert_eq!(dst, "HEAD");
}
