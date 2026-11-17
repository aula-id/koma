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

fn scratch(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "koma-picker-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn dot_slash_lists_cwd_and_keeps_the_prefix() {
    let cwd = scratch("dot");
    std::fs::create_dir_all(cwd.join("src/lib")).unwrap();
    std::fs::create_dir_all(cwd.join("target")).unwrap();
    std::fs::create_dir_all(cwd.join(".git")).unwrap();
    let top = list_dirs_in("./", &cwd, None, 10);
    assert!(top.contains(&"./src".to_string()));
    assert!(top.contains(&"./target".to_string()));
    assert!(!top.iter().any(|row| row.contains(".git")));
    assert_eq!(
        list_dirs_in("./s", &cwd, None, 10),
        vec!["./src".to_string()]
    );
    assert_eq!(
        list_dirs_in("./src/", &cwd, None, 10),
        vec!["./src/lib".to_string()]
    );
    let _ = std::fs::remove_dir_all(&cwd);
}

#[test]
fn tilde_lists_home_and_a_second_level_stays_tilde() {
    let home = scratch("home");
    let cwd = scratch("cwd");
    std::fs::create_dir_all(home.join("Documents/Projects")).unwrap();
    std::fs::create_dir_all(home.join("Downloads")).unwrap();
    std::fs::create_dir_all(home.join(".hidden")).unwrap();
    let top = list_dirs_in("~/", &cwd, Some(&home), 10);
    assert!(top.contains(&"~/Documents".to_string()));
    assert!(top.contains(&"~/Downloads".to_string()));
    assert!(!top.iter().any(|row| row.contains(".hidden")));
    assert_eq!(list_dirs_in("~", &cwd, Some(&home), 10), top);
    assert_eq!(
        list_dirs_in("~/Do", &cwd, Some(&home), 10),
        vec!["~/Documents".to_string(), "~/Downloads".to_string()]
    );
    assert_eq!(
        list_dirs_in("@~/Documents/", &cwd, Some(&home), 10),
        vec!["~/Documents/Projects".to_string()]
    );
    let docs = crate::tool::winpath::canonical(&home.join("Documents")).unwrap();
    assert_eq!(
        materialize_in("~/Documents/", &cwd, Some(&home)),
        docs.display().to_string()
    );
    let lib = cwd.join("src");
    std::fs::create_dir_all(&lib).unwrap();
    assert_eq!(
        materialize_in("./src", &cwd, Some(&home)),
        crate::tool::winpath::canonical(&lib)
            .unwrap()
            .display()
            .to_string()
    );
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&cwd);
}
