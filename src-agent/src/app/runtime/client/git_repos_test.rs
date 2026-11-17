#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "koma-gitrepos-{}-{}-{}",
        std::process::id(),
        name,
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn fake_repo(p: &Path) {
    std::fs::create_dir_all(p.join(".git")).unwrap();
}

fn scan(root: &Path, depth: usize, max_repos: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    scan_nested(
        root,
        Instant::now() + Duration::from_secs(10),
        depth,
        1000,
        max_repos,
        &mut out,
    );
    out.sort();
    out
}

#[test]
fn scan_nested_finds_container_children() {
    let r = tmp("container");
    fake_repo(&r.join("a"));
    fake_repo(&r.join("b"));
    std::fs::create_dir_all(r.join("c")).unwrap();
    assert_eq!(scan(&r, 5, 200), vec![r.join("a"), r.join("b")]);
    std::fs::remove_dir_all(&r).unwrap();
}

#[test]
fn scan_nested_finds_repo_inside_repo_and_ignores_gitignore() {
    let r = tmp("inrepo");
    fake_repo(&r);
    std::fs::write(r.join(".gitignore"), "svc/\n").unwrap();
    fake_repo(&r.join("svc"));
    assert_eq!(scan(&r, 5, 200), vec![r.join("svc")]);
    std::fs::remove_dir_all(&r).unwrap();
}

#[test]
fn scan_nested_skips_heavy_and_hidden_dirs() {
    let r = tmp("heavy");
    for d in [
        "node_modules/x",
        "target/y",
        ".hidden/z",
        "vendor/v",
        "NODE_MODULES/w",
    ] {
        fake_repo(&r.join(d));
    }
    assert!(scan(&r, 5, 200).is_empty());
    std::fs::remove_dir_all(&r).unwrap();
}

#[test]
fn scan_nested_depth_cap() {
    let r = tmp("depth");
    fake_repo(&r.join("a/b/c/d/e"));
    fake_repo(&r.join("p/q/r/s/t/u"));
    assert_eq!(scan(&r, 5, 200), vec![r.join("a/b/c/d/e")]);
    std::fs::remove_dir_all(&r).unwrap();
}

#[test]
fn scan_nested_repo_cap_and_deadline() {
    let r = tmp("caps");
    for i in 0..10 {
        fake_repo(&r.join(format!("r{i}")));
    }
    assert_eq!(scan(&r, 5, 3).len(), 3);
    let mut out = Vec::new();
    scan_nested(
        &r,
        Instant::now() - Duration::from_secs(1),
        5,
        1000,
        200,
        &mut out,
    );
    assert!(out.is_empty());
    std::fs::remove_dir_all(&r).unwrap();
}

#[cfg(unix)]
#[test]
fn scan_nested_does_not_follow_symlinks() {
    let r = tmp("symlink");
    let outside = tmp("symlink-outside");
    fake_repo(&outside.join("repo"));
    std::os::unix::fs::symlink(&outside, r.join("link")).unwrap();
    assert!(scan(&r, 5, 200).is_empty());
    std::fs::remove_dir_all(&r).unwrap();
    std::fs::remove_dir_all(&outside).unwrap();
}

#[test]
fn scan_nested_marks_gitfile_repos() {
    let r = tmp("gitfile");
    std::fs::create_dir_all(r.join("w")).unwrap();
    std::fs::write(r.join("w/.git"), "gitdir: /x/.git/worktrees/w\n").unwrap();
    assert_eq!(scan(&r, 5, 200), vec![r.join("w")]);
    std::fs::remove_dir_all(&r).unwrap();
}

#[test]
fn classify_gitdir_text_cases() {
    assert_eq!(
        classify_gitdir_text("gitdir: /r/.git/worktrees/x"),
        RepoKind::Worktree
    );
    assert_eq!(
        classify_gitdir_text("gitdir: ../../.git/modules/m"),
        RepoKind::Submodule
    );
    assert_eq!(
        classify_gitdir_text("gitdir: /r/.git/modules/m/worktrees/w"),
        RepoKind::Worktree
    );
    assert_eq!(
        classify_gitdir_text(r"gitdir: C:\r\.git\worktrees\x"),
        RepoKind::Worktree
    );
    assert_eq!(classify_gitdir_text("gitdir: ../elsewhere"), RepoKind::Repo);
    assert_eq!(classify_gitdir_text(""), RepoKind::Repo);
}

#[test]
fn classify_repo_reads_file() {
    let r = tmp("classify");
    std::fs::write(r.join(".git"), "gitdir: /r/.git/worktrees/x\n").unwrap();
    assert_eq!(classify_repo(&r), RepoKind::Worktree);
    let d = tmp("classify-dir");
    fake_repo(&d);
    assert_eq!(classify_repo(&d), RepoKind::Repo);
    std::fs::remove_dir_all(&r).unwrap();
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn rel_label_cases() {
    assert_eq!(
        rel_label(Path::new("/w"), Path::new("/w/services/api")),
        "services/api"
    );
    assert_eq!(rel_label(Path::new("/w"), Path::new("/w")), ".");
    assert_eq!(rel_label(Path::new("/w"), Path::new("/other/top")), "top");
}

#[test]
fn strip_verbatim_prefix_cases() {
    assert_eq!(
        strip_verbatim_prefix(r"\\?\C:\Users\me\proj"),
        r"C:\Users\me\proj"
    );
    assert_eq!(
        strip_verbatim_prefix(r"\\?\UNC\srv\share\p"),
        r"\\srv\share\p"
    );
    assert_eq!(strip_verbatim_prefix(r"C:\plain"), r"C:\plain");
    assert_eq!(strip_verbatim_prefix("/unix/path"), "/unix/path");
    assert_eq!(strip_verbatim_prefix(""), "");
    assert_eq!(strip_verbatim_prefix(r"\\?\"), "");
}

fn git_init(dir: &Path) -> bool {
    std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn discover_ordering_and_dedup() {
    let r = tmp("discover");
    if !git_init(&r) {
        std::fs::remove_dir_all(&r).unwrap();
        return;
    }
    std::fs::write(r.join(".gitignore"), "zeta/\n").unwrap();
    fake_repo(&r.join("zeta"));
    fake_repo(&r.join("Alpha"));
    fake_repo(&r.join("mid"));
    let infos = discover_repos_uncached(&[r.clone(), r.clone()]);
    let canon = crate::tool::winpath::canonical(&r).unwrap();
    assert_eq!(PathBuf::from(&infos[0].root), canon);
    assert_eq!(infos[0].rel, ".");
    let rels: Vec<&str> = infos.iter().map(|i| i.rel.as_str()).collect();
    assert_eq!(rels, vec![".", "Alpha", "mid", "zeta"]);
    std::fs::remove_dir_all(&r).unwrap();
}

#[test]
fn repo_info_deserializes_without_new_fields() {
    let i: RepoInfo = serde_json::from_str(r#"{"root":"/a","name":"a"}"#).unwrap();
    assert_eq!(i.rel, "");
    assert_eq!(i.kind, RepoKind::Repo);
    assert!(serde_json::to_string(&i)
        .unwrap()
        .contains(r#""kind":"repo""#));
}

#[test]
fn cache_get_respects_ttl() {
    let t0 = Instant::now();
    let repos = vec![RepoInfo {
        root: "/a".into(),
        name: "a".into(),
        rel: ".".into(),
        kind: RepoKind::Repo,
    }];
    let mut m: RepoCache = HashMap::new();
    m.insert("s".into(), (t0, repos.clone()));
    let ttl = Duration::from_secs(30);
    assert_eq!(
        cache_get(&m, "s", t0 + Duration::from_secs(5), ttl),
        Some(repos)
    );
    assert_eq!(cache_get(&m, "s", t0 + Duration::from_secs(31), ttl), None);
    assert_eq!(cache_get(&m, "other", t0, ttl), None);
}
