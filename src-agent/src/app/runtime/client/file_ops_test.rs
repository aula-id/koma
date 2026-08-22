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

fn last_json(sink: &Arc<Mutex<Vec<String>>>) -> serde_json::Value {
    let guard = sink.lock().unwrap();
    let last = guard.last().expect("expected at least one push");
    serde_json::from_str(last).expect("push must be valid json")
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
    let (sink, push) = capture_push();

    // Create a nested file + a directory.
    assert!(file_create(
        &root_s,
        "src/hello.txt",
        "file",
        "req-create-file",
        push.as_ref(),
        &workdirs,
    ));
    let created = last_json(&sink);
    assert_eq!(created["k"], "FileCreate");
    assert_eq!(created["requestId"], "req-create-file");
    assert_eq!(created["root"], root_s);
    assert_eq!(created["path"], "src/hello.txt");
    assert!(created["error"].is_null());
    assert!(dir.join("src/hello.txt").is_file());

    assert!(file_create(
        &root_s,
        "src/nested",
        "dir",
        "req-create-dir",
        push.as_ref(),
        &workdirs,
    ));
    let created_dir = last_json(&sink);
    assert_eq!(created_dir["k"], "FileCreate");
    assert_eq!(created_dir["requestId"], "req-create-dir");
    assert!(created_dir["error"].is_null());
    assert!(dir.join("src/nested").is_dir());

    // Tree listing of src/ — dirs first, excludes nothing here.
    file_tree(&root_s, "src", "req-tree-src", push.as_ref(), &workdirs);
    let tree = last_json(&sink);
    assert_eq!(tree["k"], "FileTree");
    assert_eq!(tree["requestId"], "req-tree-src");
    assert_eq!(tree["path"], "src");
    assert!(tree["error"].is_null());
    let entries = tree["entries"].as_array().expect("entries array");
    let names: Vec<&str> = entries
        .iter()
        .map(|e| e["name"].as_str().unwrap_or(""))
        .collect();
    assert!(names.contains(&"hello.txt"));
    assert!(names.contains(&"nested"));
    // Excluded dirs must not appear even if present.
    std::fs::create_dir_all(dir.join("src/target")).unwrap();
    std::fs::create_dir_all(dir.join("src/.git")).unwrap();
    file_tree(&root_s, "src", "req-tree-excl", push.as_ref(), &workdirs);
    let tree2 = last_json(&sink);
    let names2: Vec<&str> = tree2["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap_or(""))
        .collect();
    assert!(!names2.contains(&"target"));
    assert!(!names2.contains(&".git"));

    // Rename file, echo request id on success.
    assert!(file_rename(
        &root_s,
        "src/hello.txt",
        "src/hi.txt",
        "req-rename",
        push.as_ref(),
        &workdirs,
    ));
    let renamed = last_json(&sink);
    assert_eq!(renamed["k"], "FileRename");
    assert_eq!(renamed["requestId"], "req-rename");
    assert_eq!(renamed["oldPath"], "src/hello.txt");
    assert_eq!(renamed["newPath"], "src/hi.txt");
    assert!(renamed["error"].is_null());
    assert!(!dir.join("src/hello.txt").exists());
    assert!(dir.join("src/hi.txt").is_file());

    // Delete renamed file.
    assert!(file_delete(
        &root_s,
        "src/hi.txt",
        "req-delete",
        push.as_ref(),
        &workdirs,
    ));
    let deleted = last_json(&sink);
    assert_eq!(deleted["k"], "FileDelete");
    assert_eq!(deleted["requestId"], "req-delete");
    assert_eq!(deleted["path"], "src/hi.txt");
    assert!(deleted["error"].is_null());
    assert!(!dir.join("src/hi.txt").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_ops_error_replies_echo_request_ids() {
    let (dir, root_s, workdirs) = temp_workspace("errs");
    let (sink, push) = capture_push();

    // Create collision.
    std::fs::write(dir.join("exists.txt"), b"x").unwrap();
    assert!(!file_create(
        &root_s,
        "exists.txt",
        "file",
        "req-exists",
        push.as_ref(),
        &workdirs,
    ));
    let v = last_json(&sink);
    assert_eq!(v["k"], "FileCreate");
    assert_eq!(v["requestId"], "req-exists");
    assert_eq!(v["error"], "path already exists");

    // Unknown kind.
    assert!(!file_create(
        &root_s,
        "weird",
        "symlink",
        "req-kind",
        push.as_ref(),
        &workdirs,
    ));
    let v = last_json(&sink);
    assert_eq!(v["requestId"], "req-kind");
    assert!(v["error"].as_str().unwrap_or("").contains("unknown kind"));

    // Rename missing source.
    assert!(!file_rename(
        &root_s,
        "missing.txt",
        "other.txt",
        "req-ren-miss",
        push.as_ref(),
        &workdirs,
    ));
    let v = last_json(&sink);
    assert_eq!(v["k"], "FileRename");
    assert_eq!(v["requestId"], "req-ren-miss");
    assert_eq!(v["error"], "source path does not exist");

    // Rename destination collision.
    std::fs::write(dir.join("a.txt"), b"a").unwrap();
    std::fs::write(dir.join("b.txt"), b"b").unwrap();
    assert!(!file_rename(
        &root_s,
        "a.txt",
        "b.txt",
        "req-ren-coll",
        push.as_ref(),
        &workdirs,
    ));
    let v = last_json(&sink);
    assert_eq!(v["requestId"], "req-ren-coll");
    assert_eq!(v["error"], "destination already exists");

    // Delete missing + refuse workspace root.
    assert!(!file_delete(
        &root_s,
        "nope.txt",
        "req-del-miss",
        push.as_ref(),
        &workdirs,
    ));
    let v = last_json(&sink);
    assert_eq!(v["requestId"], "req-del-miss");
    assert_eq!(v["error"], "path does not exist");

    assert!(!file_delete(
        &root_s,
        "",
        "req-del-root",
        push.as_ref(),
        &workdirs,
    ));
    let v = last_json(&sink);
    assert_eq!(v["requestId"], "req-del-root");
    assert_eq!(v["error"], "refusing to delete workspace root");

    // Path escape is rejected with a push error (not a panic).
    file_tree(
        &root_s,
        "../escape",
        "req-tree-esc",
        push.as_ref(),
        &workdirs,
    );
    let v = last_json(&sink);
    assert_eq!(v["k"], "FileTree");
    assert_eq!(v["requestId"], "req-tree-esc");
    assert!(!v["error"].is_null());

    // Unconfigured root.
    let unconfigured = std::env::temp_dir()
        .join("koma-unconfigured-root")
        .to_string_lossy()
        .into_owned();
    assert!(!file_create(
        &unconfigured,
        "x.txt",
        "file",
        "req-bad-root",
        push.as_ref(),
        &workdirs,
    ));
    let v = last_json(&sink);
    assert_eq!(v["requestId"], "req-bad-root");
    assert_eq!(v["error"], "workspace root is not a configured workdir");

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

    let _ = std::fs::remove_dir_all(&dir);
}
