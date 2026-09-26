#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::io::Write;
use std::sync::{Arc, Mutex};

fn temp_workspace(tag: &str) -> (PathBuf, String, Vec<PathBuf>) {
    let dir = std::env::temp_dir().join(format!(
        "koma-fileops-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.canonicalize().unwrap();
    let root_s = root.to_string_lossy().into_owned();
    let workdirs = vec![root.clone()];
    (dir, root_s, workdirs)
}

type PushSink = Arc<Mutex<Vec<String>>>;
type PushFn = Box<dyn Fn(String)>;

fn capture_push() -> (PushSink, PushFn) {
    let sink = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink2 = Arc::clone(&sink);
    let push = Box::new(move |json: String| {
        sink2.lock().unwrap().push(json);
    });
    (sink, push)
}

fn json_for_request(sink: &Arc<Mutex<Vec<String>>>, request_id: &str) -> serde_json::Value {
    let guard = sink.lock().unwrap();
    guard
        .iter()
        .rev()
        .map(|json| {
            serde_json::from_str::<serde_json::Value>(json).expect("push must be valid json")
        })
        .find(|env| env["requestId"] == request_id)
        .expect("expected push for request id")
}

#[test]
fn fingerprint_is_stable_for_unchanged_file() {
    let dir = std::env::temp_dir().join(format!("koma-fileops-fp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.txt");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "hello fingerprint").unwrap();
    }
    let a = compute_fingerprint(&path);
    let b = compute_fingerprint(&path);
    assert_eq!(a, b);
    assert_eq!(a.len(), 16);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fingerprint_changes_when_content_changes() {
    let dir = std::env::temp_dir().join(format!("koma-fileops-fp2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("b.txt");
    std::fs::write(&path, b"one").unwrap();
    let a = compute_fingerprint(&path);
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, b"two").unwrap();
    let b = compute_fingerprint(&path);
    assert_ne!(a, b);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn safe_join_rejects_traversal_and_absolute() {
    let root = PathBuf::from("/tmp/ws");
    assert!(safe_join(&root, "src/main.rs").is_some());
    assert!(safe_join(&root, "").is_some());
    assert!(safe_join(&root, "a/./b").is_some());
    assert!(safe_join(&root, "../escape").is_none());
    assert!(safe_join(&root, "a/../../escape").is_none());
    assert!(safe_join(&root, "/etc/passwd").is_none());
}

#[test]
fn resolve_contained_rejects_escape() {
    let dir = std::env::temp_dir().join(format!("koma-fileops-esc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.canonicalize().unwrap();
    let workdirs = vec![root.clone()];
    let root_s = root.to_string_lossy().into_owned();

    assert!(resolve_contained(&root_s, "ok.txt", &workdirs).is_ok());
    assert!(resolve_contained(&root_s, "../escape", &workdirs).is_err());
    assert!(resolve_contained(&root_s, "/etc/passwd", &workdirs).is_err());
    assert!(resolve_contained("/not/a/configured/root", "x", &workdirs).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tree_sort_dirs_first_then_alpha() {
    let mut entries = vec![
        PushFileTreeEntry {
            name: "z.txt".into(),
            path: "z.txt".into(),
            is_dir: false,
        },
        PushFileTreeEntry {
            name: "B".into(),
            path: "B".into(),
            is_dir: true,
        },
        PushFileTreeEntry {
            name: "a.txt".into(),
            path: "a.txt".into(),
            is_dir: false,
        },
        PushFileTreeEntry {
            name: "A".into(),
            path: "A".into(),
            is_dir: true,
        },
    ];
    sort_entries(&mut entries);
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["A", "B", "a.txt", "z.txt"]);
}

#[test]
fn excluded_dirs_match() {
    assert!(is_excluded_dir(".git"));
    assert!(is_excluded_dir("node_modules"));
    assert!(is_excluded_dir("target"));
    assert!(is_excluded_dir(".koma"));
    assert!(!is_excluded_dir("src"));
}

#[test]
fn file_create_tree_rename_delete_roundtrip_in_temp_workspace() {
    let (dir, root_s, workdirs) = temp_workspace("crud");

    // Create a nested file + a directory.
    let created = exec_file_create(
        &root_s,
        "src/hello.txt",
        "file",
        "req-create-file",
        &workdirs,
    );
    assert!(created.mutated);
    assert!(created.error.is_none());
    assert_eq!(created.request_id, "req-create-file");
    assert_eq!(created.root, root_s);
    assert_eq!(created.path, "src/hello.txt");
    assert!(dir.join("src/hello.txt").is_file());

    let created_dir = exec_file_create(&root_s, "src/nested", "dir", "req-create-dir", &workdirs);
    assert!(created_dir.mutated);
    assert!(created_dir.error.is_none());
    assert_eq!(created_dir.request_id, "req-create-dir");
    assert!(dir.join("src/nested").is_dir());

    // Tree listing of src/ — dirs first, excludes nothing here.
    let tree = exec_file_tree(&root_s, "src", "req-tree-src", &workdirs);
    assert_eq!(tree.request_id, "req-tree-src");
    assert_eq!(tree.path, "src");
    assert!(tree.error.is_none());
    let names: Vec<&str> = tree.entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"hello.txt"));
    assert!(names.contains(&"nested"));
    // Excluded dirs must not appear even if present.
    std::fs::create_dir_all(dir.join("src/target")).unwrap();
    std::fs::create_dir_all(dir.join("src/.git")).unwrap();
    let tree2 = exec_file_tree(&root_s, "src", "req-tree-excl", &workdirs);
    let names2: Vec<&str> = tree2.entries.iter().map(|e| e.name.as_str()).collect();
    assert!(!names2.contains(&"target"));
    assert!(!names2.contains(&".git"));

    // Rename file, echo request id on success.
    let renamed = exec_file_rename(
        &root_s,
        "src/hello.txt",
        "src/hi.txt",
        "req-rename",
        &workdirs,
    );
    assert!(renamed.mutated);
    assert!(renamed.error.is_none());
    assert_eq!(renamed.request_id, "req-rename");
    assert_eq!(renamed.old_path, "src/hello.txt");
    assert_eq!(renamed.new_path, "src/hi.txt");
    assert!(!dir.join("src/hello.txt").exists());
    assert!(dir.join("src/hi.txt").is_file());

    // Delete renamed file.
    let deleted = exec_file_delete(&root_s, "src/hi.txt", "req-delete", &workdirs);
    assert!(deleted.mutated);
    assert!(deleted.error.is_none());
    assert_eq!(deleted.request_id, "req-delete");
    assert_eq!(deleted.path, "src/hi.txt");
    assert!(!dir.join("src/hi.txt").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_ops_error_replies_echo_request_ids() {
    let (dir, root_s, workdirs) = temp_workspace("errs");

    // Create collision.
    std::fs::write(dir.join("exists.txt"), b"x").unwrap();
    let v = exec_file_create(&root_s, "exists.txt", "file", "req-exists", &workdirs);
    assert!(!v.mutated);
    assert_eq!(v.request_id, "req-exists");
    assert_eq!(v.error.as_deref(), Some("path already exists"));

    // Unknown kind.
    let v = exec_file_create(&root_s, "weird", "symlink", "req-kind", &workdirs);
    assert!(!v.mutated);
    assert_eq!(v.request_id, "req-kind");
    assert!(v.error.as_deref().unwrap_or("").contains("unknown kind"));

    // Rename missing source.
    let v = exec_file_rename(
        &root_s,
        "missing.txt",
        "other.txt",
        "req-ren-miss",
        &workdirs,
    );
    assert!(!v.mutated);
    assert_eq!(v.request_id, "req-ren-miss");
    assert_eq!(v.error.as_deref(), Some("source path does not exist"));

    // Rename destination collision.
    std::fs::write(dir.join("a.txt"), b"a").unwrap();
    std::fs::write(dir.join("b.txt"), b"b").unwrap();
    let v = exec_file_rename(&root_s, "a.txt", "b.txt", "req-ren-coll", &workdirs);
    assert!(!v.mutated);
    assert_eq!(v.request_id, "req-ren-coll");
    assert_eq!(v.error.as_deref(), Some("destination already exists"));

    // Delete missing + refuse workspace root.
    let v = exec_file_delete(&root_s, "nope.txt", "req-del-miss", &workdirs);
    assert!(!v.mutated);
    assert_eq!(v.request_id, "req-del-miss");
    assert_eq!(v.error.as_deref(), Some("path does not exist"));

    let v = exec_file_delete(&root_s, "", "req-del-root", &workdirs);
    assert!(!v.mutated);
    assert_eq!(v.request_id, "req-del-root");
    assert_eq!(
        v.error.as_deref(),
        Some("refusing to delete workspace root")
    );

    // Path escape is rejected with an error (not a panic).
    let v = exec_file_tree(&root_s, "../escape", "req-tree-esc", &workdirs);
    assert_eq!(v.request_id, "req-tree-esc");
    assert!(v.error.is_some());

    // Unconfigured root.
    let unconfigured = std::env::temp_dir()
        .join("koma-unconfigured-root")
        .to_string_lossy()
        .into_owned();
    let v = exec_file_create(&unconfigured, "x.txt", "file", "req-bad-root", &workdirs);
    assert!(!v.mutated);
    assert_eq!(v.request_id, "req-bad-root");
    assert_eq!(
        v.error.as_deref(),
        Some("workspace root is not a configured workdir")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn handle_file_ctl_routes_create_tree_rename_delete() {
    let (dir, root_s, workdirs) = temp_workspace("ctl");
    let (sink, push) = capture_push();

    handle_file_ctl(
        &HostCtl::FileCreate {
            root: root_s.clone(),
            path: "note.md".into(),
            kind: "file".into(),
            request_id: "ctl-c".into(),
        },
        push.as_ref(),
        &workdirs,
        None,
    );
    assert_eq!(json_for_request(&sink, "ctl-c")["requestId"], "ctl-c");
    assert!(dir.join("note.md").is_file());

    handle_file_ctl(
        &HostCtl::FileTree {
            root: root_s.clone(),
            path: "".into(),
            request_id: "ctl-t".into(),
        },
        push.as_ref(),
        &workdirs,
        None,
    );
    let tree = json_for_request(&sink, "ctl-t");
    assert_eq!(tree["requestId"], "ctl-t");
    assert!(tree["error"].is_null());

    handle_file_ctl(
        &HostCtl::FileRename {
            root: root_s.clone(),
            old_path: "note.md".into(),
            new_path: "readme.md".into(),
            request_id: "ctl-r".into(),
        },
        push.as_ref(),
        &workdirs,
        None,
    );
    assert_eq!(json_for_request(&sink, "ctl-r")["requestId"], "ctl-r");
    assert!(dir.join("readme.md").is_file());

    handle_file_ctl(
        &HostCtl::FileDelete {
            root: root_s.clone(),
            path: "readme.md".into(),
            request_id: "ctl-d".into(),
        },
        push.as_ref(),
        &workdirs,
        None,
    );
    assert_eq!(json_for_request(&sink, "ctl-d")["requestId"], "ctl-d");
    assert!(!dir.join("readme.md").exists());

    // Binary write + download round-trip (drag-upload / save-as path).
    use base64::Engine as _;
    let payload = b"hello\0binary\xff";
    let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
    handle_file_ctl(
        &HostCtl::FileWriteBytes {
            root: root_s.clone(),
            path: "blob.bin".into(),
            bytes_b64: b64,
            overwrite: false,
            request_id: "ctl-w".into(),
        },
        push.as_ref(),
        &workdirs,
        None,
    );
    let w = json_for_request(&sink, "ctl-w");
    assert!(w["error"].is_null(), "{w}");
    assert_eq!(std::fs::read(dir.join("blob.bin")).unwrap(), payload);

    handle_file_ctl(
        &HostCtl::FileDownloadBytes {
            root: root_s.clone(),
            path: "blob.bin".into(),
            request_id: "ctl-dl".into(),
            save_as: false,
        },
        push.as_ref(),
        &workdirs,
        None,
    );
    let dl = json_for_request(&sink, "ctl-dl");
    assert!(dl["error"].is_null(), "{dl}");
    assert_eq!(dl["tooLarge"], false);
    assert_eq!(dl["size"], payload.len() as u64);
    let got = base64::engine::general_purpose::STANDARD
        .decode(dl["bytesB64"].as_str().unwrap())
        .unwrap();
    assert_eq!(got, payload);

    let _ = std::fs::remove_dir_all(&dir);
}

// Exercise the file-operation boundary, including the LF buffer sent by Monaco.
#[test]
fn coding_save_preserves_utf8_line_endings_and_bom() {
    let (dir, root, workdirs) = temp_workspace("text-formats");
    let cases: &[(&[u8], &str, &[u8])] = &[
        (b"one\r\ntwo\r\n", "one\nchanged\n", b"one\r\nchanged\r\n"),
        (b"one\ntwo", "one\nchanged", b"one\nchanged"),
        (b"one\rtwo\r", "one\nchanged\n", b"one\rchanged\r"),
        (
            b"\xef\xbb\xbfone\r\ntwo",
            "one\nchanged",
            b"\xef\xbb\xbfone\r\nchanged",
        ),
        (b"\xef\xbb\xbf", "new", b"\xef\xbb\xbfnew"),
    ];
    for (original, edited, expected) in cases {
        let path = dir.join("source.txt");
        std::fs::write(&path, original).unwrap();
        let read = exec_file_read(&root, "source.txt", "read", &workdirs);
        assert!(read.error.is_none());
        assert!(!read.content.as_ref().unwrap().contains('\r'));
        assert!(!read.content.as_ref().unwrap().starts_with('\u{feff}'));
        let saved = exec_file_save(
            &root,
            "source.txt",
            edited,
            &read.fingerprint,
            "save",
            &workdirs,
        );
        assert!(saved.error.is_none(), "{:?}", saved.error);
        assert_eq!(std::fs::read(&path).unwrap(), *expected);
        assert_eq!(
            saved.fingerprint,
            exec_file_read(&root, "source.txt", "reread", &workdirs).fingerprint
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn coding_save_preserves_utf16_byte_order_and_unicode() {
    let (dir, root, workdirs) = temp_workspace("utf16");
    for little in [true, false] {
        let encode = |text: &str| {
            let mut bytes = if little {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            bytes
        };
        let path = dir.join("source.txt");
        std::fs::write(&path, encode("α🚀\r\nbefore")).unwrap();
        let read = exec_file_read(&root, "source.txt", "read", &workdirs);
        assert_eq!(read.content.as_deref(), Some("α🚀\nbefore"));
        assert!(!read.binary);
        let saved = exec_file_save(
            &root,
            "source.txt",
            "α🚀\nafter",
            &read.fingerprint,
            "save",
            &workdirs,
        );
        assert!(saved.error.is_none(), "{:?}", saved.error);
        assert_eq!(std::fs::read(&path).unwrap(), encode("α🚀\r\nafter"));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn coding_refuses_lossy_text_conversion() {
    let (dir, root, workdirs) = temp_workspace("unsupported-text");
    for bytes in [
        b"caf\xe9".as_slice(),
        &[0xff, 0xfe, 0x00, 0xd8],
        b"one\r\ntwo\nthree",
    ] {
        let path = dir.join("source.txt");
        std::fs::write(&path, bytes).unwrap();
        let read = exec_file_read(&root, "source.txt", "read", &workdirs);
        assert!(read.content.is_none());
        assert!(read.error.is_some());
        // Even a caller with a current fingerprint cannot force a lossy write.
        let saved = exec_file_save(
            &root,
            "source.txt",
            "replacement",
            &compute_fingerprint(&path),
            "save",
            &workdirs,
        );
        assert!(saved.error.is_some());
        assert!(!saved.mutated);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn coding_stale_save_keeps_externally_changed_bytes() {
    let (dir, root, workdirs) = temp_workspace("stale-save");
    let path = dir.join("source.txt");
    std::fs::write(&path, b"before\r\n").unwrap();
    let read = exec_file_read(&root, "source.txt", "read", &workdirs);
    std::fs::write(&path, b"external\r\n").unwrap();
    let saved = exec_file_save(
        &root,
        "source.txt",
        "local\n",
        &read.fingerprint,
        "save",
        &workdirs,
    );
    assert!(saved.error.as_deref().unwrap().starts_with("conflict:"));
    assert_eq!(std::fs::read(&path).unwrap(), b"external\r\n");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[cfg(unix)]
fn atomic_save_preserves_symlink_and_mode_and_rejects_hardlinks() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let (dir, root, workdirs) = temp_workspace("atomic-save");
    let target = dir.join("target.txt");
    std::fs::write(&target, b"old\n").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    symlink("target.txt", dir.join("link.txt")).unwrap();
    let read = exec_file_read(&root, "link.txt", "read", &workdirs);
    let saved = exec_file_save(&root, "link.txt", "new\n", &read.fingerprint, "save", &workdirs);
    assert!(saved.error.is_none(), "{:?}", saved.error);
    assert!(dir.join("link.txt").symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new\n");
    assert_eq!(target.metadata().unwrap().permissions().mode() & 0o777, 0o640);
    std::fs::hard_link(&target, dir.join("hard.txt")).unwrap();
    let rejected = exec_file_save(&root, "target.txt", "replacement\n", &saved.fingerprint, "hard", &workdirs);
    assert!(rejected.error.as_deref().unwrap_or("").contains("hard links"));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new\n");
    assert!(!std::fs::read_dir(&dir).unwrap().flatten().any(|entry| entry.file_name().to_string_lossy().starts_with(".koma-write-")));
    std::fs::remove_dir_all(dir).unwrap();
}
