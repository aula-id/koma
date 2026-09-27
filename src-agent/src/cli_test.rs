use super::parse;

#[test]
fn run_current_drss_and_extension_flags() {
    let run = parse(
        [
            "koma",
            "--session",
            "existing",
            "run",
            "--status",
            "--short-send",
            "off",
            "--max-tokens",
            "0",
            "--context-window-limit",
            "1000000",
            "--context-model-alias",
            "model-alias",
            "--extension",
            "run.koma.one",
            "--extension",
            "run.koma.two",
            "--extension",
            "run.koma.one",
            "--unload-extension",
            "run.koma.old",
        ]
        .into_iter()
        .map(String::from),
    )
    .run
    .unwrap();
    assert!(run.error.is_none(), "{:?}", run.error);
    assert_eq!(run.session.as_deref(), Some("existing"));
    assert!(run.status);
    assert_eq!(run.short_send, Some(false));
    assert_eq!(run.max_tokens, Some(0));
    assert_eq!(run.context_window_limit, Some(300_000));
    assert_eq!(run.context_model_alias.as_deref(), Some("model-alias"));
    assert_eq!(run.extensions, vec!["run.koma.one", "run.koma.two"]);
    assert_eq!(run.unload_extensions, vec!["run.koma.old"]);
}

#[test]
fn run_rejects_invalid_and_retired_setup_flags() {
    for args in [
        vec!["--short-send", "maybe"],
        vec!["--context-window-limit", "-1"],
        vec!["--max-tokens", "wrong"],
        vec!["--extension"],
        vec!["--mode", "made-up"],
        vec!["--short-send-engage-n", "80"],
        vec!["--mode", "yolo", "--security", "off"],
        vec![
            "--extension",
            "run.koma.one",
            "--unload-extension",
            "run.koma.one",
        ],
    ] {
        let run = parse(
            ["koma", "run"]
                .into_iter()
                .chain(args.iter().copied())
                .map(String::from),
        )
        .run
        .unwrap();
        assert!(
            run.error.is_some(),
            "invalid args silently accepted: {args:?}"
        );
    }
}

#[test]
fn run_flags_before_verb_and_prompt_flag_literals_are_not_misparsed() {
    let run = parse(
        [
            "koma",
            "--short-send",
            "on",
            "run",
            "--prompt",
            "--extension",
        ]
        .into_iter()
        .map(String::from),
    )
    .run
    .unwrap();
    assert!(run.error.is_none());
    assert_eq!(run.short_send, Some(true));
    assert_eq!(run.prompt.as_deref(), Some("--extension"));
    assert!(run.extensions.is_empty());
}

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
fn run_model_effort_mode_security_flags() {
    let opts = parse(
        [
            "koma",
            "run",
            "--prompt",
            "go",
            "--model",
            "laguna-s-2.1",
            "--effort",
            "off",
            "--mode",
            "yolo",
            "--security",
            "on",
            "--once",
        ]
        .into_iter()
        .map(String::from),
    );
    let run = opts.run.expect("run cli");
    assert_eq!(run.model.as_deref(), Some("laguna-s-2.1"));
    assert_eq!(run.effort.as_deref(), Some("off"));
    assert_eq!(run.mode.as_deref(), Some("yolo"));
    assert_eq!(run.security, Some(true));
    assert!(run.once);
}

#[test]
fn run_system_and_system_file_flags() {
    let inline = parse(
        [
            "koma",
            "run",
            "--prompt",
            "go",
            "--system",
            "task_dir is the desk",
        ]
        .into_iter()
        .map(String::from),
    );
    let run = inline.run.expect("run cli");
    assert_eq!(run.system.as_deref(), Some("task_dir is the desk"));
    assert!(run.system_file.is_none());

    let file = parse(
        [
            "koma",
            "run",
            "--prompt-file",
            "/tmp/p.txt",
            "--system-file",
            "/tmp/sys.txt",
        ]
        .into_iter()
        .map(String::from),
    );
    let run = file.run.expect("run cli");
    assert_eq!(run.system_file.as_deref(), Some("/tmp/sys.txt"));
    assert!(run.system.is_none());
}

#[test]
fn run_system_and_system_file_are_exclusive() {
    let opts = parse(
        [
            "koma",
            "run",
            "--prompt",
            "go",
            "--system",
            "a",
            "--system-file",
            "/tmp/s.txt",
        ]
        .into_iter()
        .map(String::from),
    );
    let run = opts.run.expect("run cli");
    assert!(run.error.as_deref().unwrap().contains("--system"));
}

#[test]
fn run_max_tokens_flag() {
    let opts = parse(
        ["koma", "run", "--prompt", "go", "--max-tokens", "8192"]
            .into_iter()
            .map(String::from),
    );
    let run = opts.run.expect("run cli");
    assert_eq!(run.max_tokens, Some(8192));
}

#[test]
fn run_security_off_parses() {
    let opts = parse(
        ["koma", "run", "--prompt", "x", "--security", "off"]
            .into_iter()
            .map(String::from),
    );
    let run = opts.run.expect("run cli");
    assert_eq!(run.security, Some(false));
}

#[test]
fn parse_on_off_tokens() {
    assert_eq!(crate::cli::parse_on_off("ON"), Some(true));
    assert_eq!(crate::cli::parse_on_off("false"), Some(false));
    assert_eq!(crate::cli::parse_on_off("maybe"), None);
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
