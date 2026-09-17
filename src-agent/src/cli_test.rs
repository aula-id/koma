use super::parse;

#[test]
fn bare_remote_opens_saved_host_picker() {
    let opts = parse(["koma", "remote"].into_iter().map(String::from));
    assert!(opts.remote_picker);
    assert!(opts.remote_target.is_none());
}

#[test]
fn remote_target_uses_direct_remote_entry() {
    let opts = parse(
        ["koma", "remote", "alice@example.test"]
            .into_iter()
            .map(String::from),
    );
    assert!(!opts.remote_picker);
    assert_eq!(opts.remote_target.as_deref(), Some("alice@example.test"));
}

#[test]
fn lsp_status_subcommand() {
    let opts = parse(["koma", "lsp", "status"].into_iter().map(String::from));
    match opts.lsp {
        Some(crate::lsp::LspCli::Status) => {}
        other => panic!("expected Status, got {other:?}"),
    }
}

#[test]
fn lsp_install_all_force() {
    let opts = parse(
        ["koma", "lsp", "install", "--all", "--force"]
            .into_iter()
            .map(String::from),
    );
    match opts.lsp {
        Some(crate::lsp::LspCli::Install {
            id: None,
            all: true,
            force: true,
        }) => {}
        other => panic!("expected install --all --force, got {other:?}"),
    }
}

#[test]
fn run_inline_prompt_once_timeout() {
    let opts = parse(
        [
            "koma",
            "run",
            "--name",
            "cg-10400",
            "--prompt",
            "hello",
            "--once",
            "--timeout",
            "30",
            "--workdir",
            "/tmp",
        ]
        .into_iter()
        .map(String::from),
    );
    let run = opts.run.expect("run cli");
    assert_eq!(run.name.as_deref(), Some("cg-10400"));
    assert_eq!(run.prompt.as_deref(), Some("hello"));
    assert!(run.prompt_file.is_none());
    assert!(run.once);
    assert_eq!(run.timeout_sec, 30);
    assert_eq!(run.workdir.as_deref(), Some("/tmp"));
}

#[test]
fn run_prompt_file_flag() {
    let opts = parse(
        [
            "koma",
            "run",
            "--prompt-file",
            "/tmp/p.txt",
            "--session",
            "abc",
        ]
        .into_iter()
        .map(String::from),
    );
    let run = opts.run.expect("run cli");
    assert_eq!(run.prompt_file.as_deref(), Some("/tmp/p.txt"));
    assert!(run.prompt.is_none());
    assert_eq!(run.session.as_deref(), Some("abc"));
    assert!(!run.once);
    assert_eq!(run.timeout_sec, 14_400);
}

#[test]
fn unknown_positional_is_not_default_launch() {
    let opts = parse(["koma", "docker"].into_iter().map(String::from));
    assert_eq!(opts.unknown_command.as_deref(), Some("docker"));
    assert!(!opts.doctor);
    assert!(opts.run.is_none());
}

#[test]
fn bare_koma_has_no_unknown_command() {
    let opts = parse(["koma"].into_iter().map(String::from));
    assert!(opts.unknown_command.is_none());
}

#[test]
fn known_doctor_not_unknown() {
    let opts = parse(["koma", "doctor"].into_iter().map(String::from));
    assert!(opts.doctor);
    assert!(opts.unknown_command.is_none());
}

#[test]
fn session_flag_value_is_not_unknown_command() {
    // Regression: unknown-command routing must not treat `--session <id>`'s value
    // as a positional verb — that broke `koma --daemon --session <uuid>` spawn.
    let opts = parse(
        [
            "koma",
            "--daemon",
            "--session",
            "7d59eb3c-59e2-4ffc-a3a5-2f15e54918d6",
        ]
        .into_iter()
        .map(String::from),
    );
    assert!(opts.daemon);
    assert_eq!(
        opts.session.as_deref(),
        Some("7d59eb3c-59e2-4ffc-a3a5-2f15e54918d6")
    );
    assert!(opts.unknown_command.is_none());
}

#[test]
fn daemon_session_only_flags_no_positional_verb() {
    let opts = parse(
        ["koma", "--session", "abc-123", "--daemon"]
            .into_iter()
            .map(String::from),
    );
    assert!(opts.daemon);
    assert_eq!(opts.session.as_deref(), Some("abc-123"));
    assert!(opts.unknown_command.is_none());
    assert!(!opts.resume);
}
