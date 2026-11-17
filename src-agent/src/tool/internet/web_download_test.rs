use super::check_save_name;

#[test]
fn rejects_traversal_and_separators_on_both() {
    for w in [false, true] {
        for bad in ["../../x", "/abs", "a/b", "..", ".", "a\\b"] {
            assert!(check_save_name(bad, w).is_err(), "{bad:?} windows={w}");
        }
    }
}

#[test]
fn windows_only_rules() {
    for bad in ["C:evil.txt", "a:s", "nul", "Con.txt"] {
        assert!(check_save_name(bad, true).is_err(), "{bad:?}");
        assert!(check_save_name(bad, false).is_ok(), "{bad:?}");
    }
}

#[test]
fn accepts_plain_names() {
    for w in [false, true] {
        assert!(check_save_name("report-1.pdf", w).is_ok());
        assert!(check_save_name("wt_1", w).is_ok());
    }
}
