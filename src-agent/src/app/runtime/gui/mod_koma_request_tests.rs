#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// A well-formed reverse-DNS id passes; anything that could escape `extensions_dir()`
/// as a directory name (empty, `.`/`..`, embedded `/`) is rejected.
#[test]
fn safe_ext_id_rejects_path_escapes() {
    assert_eq!(
        safe_ext_id("run.koma.example.fleet-board-daemon"),
        Some("run.koma.example.fleet-board-daemon")
    );
    assert_eq!(safe_ext_id(""), None);
    assert_eq!(safe_ext_id("."), None);
    assert_eq!(safe_ext_id(".."), None);
    assert_eq!(safe_ext_id("../etc"), None);
    assert_eq!(safe_ext_id("a/b"), None);
    assert_eq!(safe_ext_id(".hidden"), None);
}

/// A plain relative asset path resolves cleanly; `..` (any position), an absolute
/// path, or an embedded backslash is rejected outright — the zip-slip-style guard
/// applied to protocol request paths instead of zip entries.
#[test]
fn safe_ext_rel_path_rejects_escapes() {
    assert_eq!(
        safe_ext_rel_path_for("index.html", false),
        Some(PathBuf::from("index.html"))
    );
    assert_eq!(
        safe_ext_rel_path_for("assets/index-abc123.js", false),
        Some(PathBuf::from("assets/index-abc123.js"))
    );
    assert_eq!(safe_ext_rel_path_for("", false), Some(PathBuf::new()));
    assert_eq!(safe_ext_rel_path_for("..", false), None);
    assert_eq!(safe_ext_rel_path_for("../../etc/passwd", false), None);
    assert_eq!(safe_ext_rel_path_for("assets/../../escape", false), None);
    assert_eq!(safe_ext_rel_path_for("/etc/passwd", false), None);
    assert_eq!(safe_ext_rel_path_for("a\\b", false), None);
}

fn write_ext_file(dir: &Path, rel: &str, bytes: &[u8]) {
    let p = dir.join("com.x.demo").join("ui").join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("koma-gui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn extension_requests_decode_once_and_never_escape() {
    let dir = temp_dir("ext");
    write_ext_file(&dir, "my icon.png", b"png");
    std::fs::write(dir.join("com.x.demo").join("secret"), b"s").unwrap();
    std::fs::write(dir.join("com.x.demo").join("ui").join("%2e%2e"), b"literal").unwrap();
    let get = |p: &str| serve_extension_file(&dir, p, false);
    let ok = get("/com.x.demo/my%20icon.png");
    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(ok.headers()["Content-Type"], "image/png");
    assert_eq!(
        get("/com.x.demo/%2e%2e/secret").status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get("/com.x.demo/a%5c..%5cb").status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get("/com.x.demo/%2f..%2fsecret").status(),
        StatusCode::NOT_FOUND
    );
    // double-encoded: decoded once to a literal `%2e%2e/x`, which is not a traversal
    assert_eq!(
        get("/com.x.demo/%252e%252e/x").status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(get("/com.x.demo/%ff").status(), StatusCode::NOT_FOUND);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn image_route_serves_only_allowlisted_extensions() {
    let dir = temp_dir("img");
    for n in ["a.png", "b.PNG", "id_rsa", "x.txt", ".pem"] {
        std::fs::write(dir.join(n), b"data").unwrap();
    }
    let req = |n: &str| {
        let enc = dir.join(n).to_string_lossy().replace(' ', "%20");
        let uri = format!("koma://localhost/image/{}", enc.trim_start_matches('/'));
        let enc = if enc.starts_with('/') {
            format!(
                "koma://localhost/image/%2F{}",
                enc.trim_start_matches('/').replace('/', "%2F")
            )
        } else {
            uri
        };
        handle_koma_request(Request::builder().uri(enc).body(Vec::new()).unwrap())
    };
    for n in ["a.png", "b.PNG"] {
        let r = req(n);
        assert_eq!(r.status(), StatusCode::OK, "{n}");
        assert_eq!(r.headers()["Content-Type"], "image/png");
    }
    for n in ["id_rsa", "x.txt", ".pem"] {
        assert_eq!(req(n).status(), StatusCode::NOT_FOUND, "{n}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mime_table_is_case_insensitive() {
    assert_eq!(mime_for("A.MJS"), "text/javascript");
    assert_eq!(mime_for("x.JPG"), "image/jpeg");
    assert_eq!(mime_for("a.wasm"), "application/wasm");
    assert_eq!(mime_for("a.bin"), "application/octet-stream");
    assert_eq!(image_mime(Path::new("a.AVIF")), Some("image/avif"));
    assert_eq!(image_mime(Path::new("id_rsa")), None);
}
