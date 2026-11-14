use super::{
    exit_multiplex, multiplex_opts, remote_command, server_args, shell_quote, validate_remote_path,
};

#[test]
fn remote_command_quotes_program_and_arguments() {
    let prog = "/home/me/.local/bin/koma";
    let dangerous = "id; echo bad";
    let cmd = remote_command(prog, &["server", "--session", dangerous]).unwrap();
    // Login+interactive bash so profile/bashrc (cargo, nvm) load; exec keeps
    // stdin for the child. Full argv is shell-quoted inside the -c payload.
    let expected_inner = format!(
        r#"[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env" 2>/dev/null; printf '\n%s\n' KOMA-STREAM-BEGIN; exec {} {} {} {}"#,
        shell_quote(prog),
        shell_quote("server"),
        shell_quote("--session"),
        shell_quote(dangerous),
    );
    assert_eq!(cmd, format!("bash -ilc {}", shell_quote(&expected_inner)));
    // Dangerous metacharacters only appear inside a single shell-quoted word.
    assert!(expected_inner.contains(&shell_quote(dangerous)));
    assert_eq!(shell_quote("/home/a'b/koma"), "'/home/a'\\''b/koma'");
    assert!(remote_command("koma", &[]).is_err());
    assert!(remote_command("/usr/bin/koma", &["bad\narg"]).is_err());
}

#[test]
fn remote_command_inner_argv_is_shell_quoted() {
    let cmd = remote_command("/usr/bin/koma", &["--version"]).unwrap();
    let expected_inner = concat!(
        r#"[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env" 2>/dev/null; printf '\n%s\n' KOMA-STREAM-BEGIN; exec "#,
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

// The command runs on the remote, which is always a POSIX shell; a Windows
// client has no /bin/sh to run it locally.
#[cfg(unix)]
#[test]
fn remote_command_output_is_readable_through_noisy_startup_files() {
    // Run the real command under a shell that prints noise first, the way a
    // chatty .bashrc does, and check the reader sees only the program output.
    let cmd = remote_command("/bin/echo", &["payload"]).expect("build remote command");
    let inner = cmd
        .strip_prefix("bash -ilc ")
        .expect("bash -ilc wrapper");
    let script = format!("echo 'Welcome to the server'; printf 'no newline'; eval {inner}");
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", &script])
        .output()
        .expect("run /bin/sh");
    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    assert_eq!(crate::remote::preamble::strip(&stdout), "payload\n");
}

#[test]
fn only_rejected_credentials_count_as_auth_failures() {
    let rejected: anyhow::Error = super::SshTransportError {
        stderr: "user@host: Permission denied (publickey,password).".into(),
    }
    .into();
    assert!(super::is_auth_rejection(&rejected.context("bootstrap")));
    let refused: anyhow::Error = super::SshTransportError {
        stderr: "ssh: connect to host h port 22: Connection refused".into(),
    }
    .into();
    assert!(!super::is_auth_rejection(&refused));
    assert!(super::is_transport_error(&refused));
    assert!(!super::is_auth_rejection(&anyhow::anyhow!("Permission denied")));
}
