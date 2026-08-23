use super::{
    exit_multiplex, multiplex_opts, remote_command, server_args, shell_quote, validate_remote_path,
};

#[test]
fn remote_command_quotes_program_and_arguments() {
    let cmd = remote_command(
        "/home/me/.local/bin/koma",
        &["server", "--session", "id; echo bad"],
    )
    .unwrap();
    // Login+interactive bash so profile/bashrc (cargo, nvm) load; exec keeps
    // stdin for the child. Argv stays single-quoted inside the -c string.
    assert!(cmd.starts_with("bash -ilc "), "{cmd}");
    assert!(cmd.contains("exec "), "{cmd}");
    assert!(cmd.contains("/home/me/.local/bin/koma"), "{cmd}");
    assert!(cmd.contains("server"), "{cmd}");
    assert!(cmd.contains("id; echo bad"), "{cmd}");
    // Metacharacters must not appear unquoted as a separate shell word.
    assert!(!cmd.contains(" echo "), "{cmd}");
    assert_eq!(shell_quote("/home/a'b/koma"), "'/home/a'\\''b/koma'");
    assert!(remote_command("koma", &[]).is_err());
    assert!(remote_command("/usr/bin/koma", &["bad\narg"]).is_err());
}

#[test]
fn remote_command_inner_argv_is_shell_quoted() {
    let cmd = remote_command("/usr/bin/koma", &["--version"]).unwrap();
    let expected_inner = concat!(
        r#"[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env" 2>/dev/null; exec "#,
        "'/usr/bin/koma' '--version'"
    );
    assert_eq!(cmd, format!("bash -ilc {}", shell_quote(expected_inner)));
}

#[test]
fn server_args_keep_cwd_as_one_argument() {
    let args = server_args("id", Some("/tmp/a; echo bad")).unwrap();
    assert_eq!(
        args,
        ["server", "--session", "id", "--cwd", "/tmp/a; echo bad"]
    );
}
#[test]
fn server_args_reject_empty_values() {
    assert!(server_args("", None).is_err());
    assert!(server_args("id", Some("")).is_err());
}

#[test]
fn remote_paths_are_trimmed_and_reject_empty_or_nul() {
    assert_eq!(validate_remote_path(" /srv/work ").unwrap(), "/srv/work");
    assert!(validate_remote_path(" ").is_err());
    assert!(validate_remote_path("/srv/\0work").is_err());
}

#[test]
fn multiplex_opts_shape_on_unix() {
    #[cfg(unix)]
    {
        let opts = multiplex_opts();
        // Either empty (base_dir failed) or the three -o pairs.
        if opts.is_empty() {
            return;
        }
        assert_eq!(opts.len(), 6);
        assert_eq!(opts[0], "-o");
        assert_eq!(opts[1], "ControlMaster=auto");
        assert_eq!(opts[2], "-o");
        assert!(opts[3].starts_with("ControlPath="));
        assert!(opts[3].contains("ssh-mux"));
        assert!(opts[3].ends_with("%C"));
        assert_eq!(opts[4], "-o");
        assert_eq!(opts[5], "ControlPersist=300");
    }
    #[cfg(not(unix))]
    {
        assert!(multiplex_opts().is_empty());
    }
}

#[test]
fn exit_multiplex_is_best_effort_noop_without_master() {
    // Must not panic when no master exists for a nonsense target.
    exit_multiplex(&crate::remote::RemoteTarget {
        user: "nobody".into(),
        host: "127.0.0.1".into(),
        port: Some(1),
        key: None,
    });
}
