use super::resolve::*;
use std::collections::HashSet;

fn files<'a>(list: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
    let set: HashSet<&str> = list.iter().copied().collect();
    move |p| set.contains(p)
}

#[test]
fn windows_bare_prefers_cmd_over_extensionless_script() {
    let f = files(&[r"C:\nodejs\npx", r"C:\nodejs\npx.cmd"]);
    let got = resolve_with("npx", r"C:\a;C:\nodejs", None, None, true, &f);
    assert_eq!(got.as_deref(), Some(r"C:\nodejs\npx.cmd"));
}

#[test]
fn windows_pathext_order_and_filtering() {
    let list = candidates(
        "x",
        r"C:\d",
        Some(".COM;.EXE;.BAT;.CMD;.VBS;.PS1"),
        None,
        true,
    );
    assert_eq!(
        list,
        vec![r"C:\d\x.com", r"C:\d\x.exe", r"C:\d\x.bat", r"C:\d\x.cmd"]
    );
    let f = files(&[r"C:\d\x.exe", r"C:\d\x.cmd"]);
    let got = resolve_with("x", r"C:\d", Some(".COM;.EXE;.CMD"), None, true, &f);
    assert_eq!(got.as_deref(), Some(r"C:\d\x.exe"));
}

#[test]
fn windows_pathext_unset_uses_default_order_case_insensitively() {
    assert_eq!(spawnable_exts(None), vec![".com", ".exe", ".bat", ".cmd"]);
    assert_eq!(
        spawnable_exts(Some(".CMD;.EXE")),
        spawnable_exts(Some(".cmd;.exe"))
    );
    assert_eq!(
        spawnable_exts(Some(".VBS")),
        vec![".com", ".exe", ".bat", ".cmd"]
    );
}

#[test]
fn windows_directory_order_beats_extension_order() {
    let f = files(&[r"C:\a\x.cmd", r"C:\b\x.exe"]);
    let got = resolve_with("x", r"C:\a;C:\b", None, None, true, &f);
    assert_eq!(got.as_deref(), Some(r"C:\a\x.cmd"));
}

#[test]
fn windows_explicit_extension_tries_exact_first() {
    let list = candidates("node.exe", r"C:\d", None, None, true);
    assert_eq!(list.first().map(String::as_str), Some(r"C:\d\node.exe"));
    let list = candidates("foo.bar", r"C:\d", None, None, true);
    assert_eq!(list.first().map(String::as_str), Some(r"C:\d\foo.bar"));
    assert_eq!(list.get(1).map(String::as_str), Some(r"C:\d\foo.bar.com"));
}

#[test]
fn windows_quoted_empty_and_trailing_separator_entries() {
    let list = candidates(
        "x",
        r#";;"C:\Program Files\nodejs"; C:\t\ ;"#,
        Some(".exe"),
        None,
        true,
    );
    assert_eq!(list, vec![r"C:\Program Files\nodejs\x.exe", r"C:\t\x.exe"]);
}

#[test]
fn windows_path_like_commands() {
    assert_eq!(
        candidates(r"C:\tools\srv", "", None, None, true)
            .first()
            .map(String::as_str),
        Some(r"C:\tools\srv.com")
    );
    assert!(
        !candidates(r"C:\tools\srv", "", None, None, true).contains(&r"C:\tools\srv".to_string())
    );
    assert_eq!(
        candidates(r".\bin\srv", "", Some(".exe"), Some(r"D:\proj"), true),
        vec![r"D:\proj\.\bin\srv.exe"]
    );
    assert_eq!(
        candidates("C:/x/y.cmd", "", Some(".exe"), None, true),
        vec!["C:/x/y.cmd", "C:/x/y.cmd.exe"]
    );
}

#[test]
fn unix_bare_and_path_like() {
    let f = files(&["/opt/node/bin/npx"]);
    let got = resolve_with("npx", "/usr/bin:/opt/node/bin", None, None, false, &f);
    assert_eq!(got.as_deref(), Some("/opt/node/bin/npx"));
    assert_eq!(
        candidates("./srv", "/usr/bin", None, Some("/p"), false),
        vec!["/p/./srv"]
    );
    assert_eq!(
        candidates("/abs/srv", "", None, Some("/p"), false),
        vec!["/abs/srv"]
    );
}

#[test]
fn not_found_resolves_to_none_and_message_mentions_details() {
    let f = files(&[]);
    assert_eq!(resolve_with("npxx", "/a:/b", None, None, false, &f), None);
    let m = not_found_message("npxx", r"C:\a;C:\b;;", None, true);
    assert!(m.starts_with("command 'npxx' was not found on PATH"));
    assert!(m.contains("2 directories"));
    assert!(m.contains(".com, .exe, .bat, .cmd"));
    let m = not_found_message("npxx", "/a:/b", None, false);
    assert!(!m.contains("tried"));
}

#[test]
fn effective_path_override_rules() {
    let env = vec![("Path".to_string(), "X".to_string())];
    assert_eq!(effective_path(&env, "H", true), "X");
    assert_eq!(effective_path(&env, "H", false), "H");
    let env = vec![("PATH".to_string(), "Y".to_string())];
    assert_eq!(effective_path(&env, "H", false), "Y");
    assert_eq!(effective_path(&[], "H", true), "H");
}

#[test]
fn choose_program_rule() {
    assert_eq!(
        choose_program("npx", Some(r"C:\n\npx.cmd"), true).as_deref(),
        Some(r"C:\n\npx.cmd")
    );
    assert_eq!(choose_program("npx", None, true), None);
    assert_eq!(
        choose_program("npx", Some("/usr/bin/npx"), false).as_deref(),
        Some("npx")
    );
    assert_eq!(choose_program("npx", None, false).as_deref(), Some("npx"));
    assert_eq!(
        choose_program("./srv", Some("/p/./srv"), false).as_deref(),
        Some("/p/./srv")
    );
    assert_eq!(
        choose_program("./srv", None, false).as_deref(),
        Some("./srv")
    );
}

#[test]
fn spawn_error_descriptions() {
    use std::io::ErrorKind::*;
    let m = describe_spawn_error("npx", r"C:\n\npx.cmd", InvalidInput, "e", true);
    assert!(m.contains("batch file"));
    let m = describe_spawn_error("npx", "/u/npx.cmd", InvalidInput, "bad", false);
    assert_eq!(m, "spawn 'npx' failed: bad");
    assert_eq!(
        describe_spawn_error("a", "/p/a", NotFound, "", false),
        "spawn '/p/a' failed: file not found"
    );
    assert!(
        describe_spawn_error("a", "/p/a", PermissionDenied, "", false)
            .contains("permission denied")
    );
}

#[test]
fn tail_keeps_last_bytes_on_char_boundary() {
    let mut buf = String::new();
    for i in 0..50 {
        push_tail(&mut buf, &format!("line {i}"), 40);
    }
    assert!(buf.len() <= 40);
    assert!(buf.ends_with("line 49\n"));
    assert!(buf.starts_with("line"));
    let mut buf = String::new();
    push_tail(&mut buf, &"é".repeat(100), 21);
    assert!(buf.len() <= 21);
    assert!(buf.ends_with('\n'));
}

#[test]
fn with_stderr_appends_only_when_present() {
    assert_eq!(with_stderr("m".into(), "  \n"), "m");
    assert_eq!(
        with_stderr("m".into(), "boom\n"),
        "m\nserver stderr (last lines):\nboom"
    );
}
