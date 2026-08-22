use super::*;

#[test]
fn is_abs_path_unix() {
    assert!(is_abs_path("/home/user"));
}

#[test]
fn is_abs_path_windows_drive() {
    assert!(is_abs_path("C:\\Users\\foo"));
}

#[test]
fn is_abs_path_windows_unc() {
    assert!(is_abs_path("\\\\server\\share"));
}

#[test]
fn is_abs_path_windows_backslash_root() {
    assert!(is_abs_path("\\Users\\foo"));
}

#[test]
fn is_abs_path_relative_slash() {
    assert!(!is_abs_path("src/main.rs"));
}

#[test]
fn is_abs_path_relative_backslash() {
    assert!(!is_abs_path("src\\main.rs"));
}

#[test]
fn last_sep_forward_slash() {
    assert_eq!(last_sep("src/main.rs"), Some(3));
}

#[test]
fn last_sep_backslash() {
    assert_eq!(last_sep("src\\main.rs"), Some(3));
}

#[test]
fn last_sep_mixed() {
    assert_eq!(last_sep("src/main\\file.rs"), Some(8));
}

#[test]
fn last_sep_no_slash() {
    assert_eq!(last_sep("README.md"), None);
}
