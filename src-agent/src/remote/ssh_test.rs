use super::{remote_command, server_args, shell_quote, validate_remote_path};
#[test]
fn remote_command_quotes_program_and_arguments() {
    assert_eq!(
        remote_command(
            "/home/me/.local/bin/koma",
            &["server", "--session", "id; echo bad"]
        )
        .unwrap(),
        "'/home/me/.local/bin/koma' 'server' '--session' 'id; echo bad'"
    );
    assert_eq!(shell_quote("/home/a'b/koma"), "'/home/a'\\''b/koma'");
    assert!(remote_command("koma", &[]).is_err());
    assert!(remote_command("/usr/bin/koma", &["bad\narg"]).is_err());
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
