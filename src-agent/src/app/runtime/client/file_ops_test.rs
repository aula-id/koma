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
