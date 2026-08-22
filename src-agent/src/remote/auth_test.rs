use super::*;

#[test]
fn askpass_script_validates_prompt() {
    let script = askpass_script_content("hunter2");
    // Should contain the password for password prompts.
    assert!(script.contains("hunter2"));
    // Should have the case statement for validation.
    assert!(script.contains("[Pp]assword*"));
    assert!(script.contains("[Pp]assphrase"));
}

#[test]
fn askpass_script_escapes_quotes() {
    let script = askpass_script_content("it's-a-secret");
    // The escaped password should handle single quotes.
    assert!(script.contains("it'\\''s-a-secret"));
}

#[cfg(unix)]
#[test]
fn askpass_helper_outputs_password_with_newline_and_rejects_other_prompts() {
    let auth = SshAuth::new("it's-a-secret".to_string()).unwrap();
    let helper = auth.askpass_path.as_ref().unwrap();

    let password_output = StdCommand::new("sh")
        .arg(helper)
        .arg("user@example.com's password:")
        .output()
        .unwrap();
    assert!(password_output.status.success());
    assert_eq!(password_output.stdout, b"it's-a-secret\n");

    let unrelated_output = StdCommand::new("sh")
        .arg(helper)
        .arg("Are you sure you want to continue connecting (yes/no)?")
        .output()
        .unwrap();
    assert!(!unrelated_output.status.success());
    assert!(unrelated_output.stdout.is_empty());
}

#[test]
fn ssh_auth_new_creates_unique_files() {
    let auth1 = SshAuth::new("pass1".to_string()).unwrap();
    let auth2 = SshAuth::new("pass2".to_string()).unwrap();
    // Both should have different askpass paths.
    assert_ne!(auth1.askpass_path, auth2.askpass_path);
    // Both files should exist.
    assert!(auth1.askpass_path.as_ref().unwrap().exists());
    assert!(auth2.askpass_path.as_ref().unwrap().exists());
}

#[test]
fn ssh_auth_from_password_works() {
    let auth = SshAuth::from_password("secret".to_string()).unwrap();
    assert!(auth.askpass_path.is_some());
    assert!(auth.askpass_path.as_ref().unwrap().exists());
}

#[test]
fn ssh_auth_debug_hides_password() {
    let auth = SshAuth::new("topsecret".to_string()).unwrap();
    let debug = format!("{auth:?}");
    assert!(
        !debug.contains("topsecret"),
        "Debug output must not contain the password"
    );
    assert!(debug.contains("has_password: true"));
    assert!(debug.contains("has_askpass: true"));
}

#[test]
fn ssh_auth_drop_cleans_up() {
    let path;
    {
        let auth = SshAuth::new("cleanup-test".to_string()).unwrap();
        path = auth.askpass_path.clone().unwrap();
        assert!(path.exists());
    }
    // After drop, the file should be deleted. Retry briefly to handle
    // filesystem race on slow/parallel CI runners.
    for _ in 0..10 {
        if !path.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!path.exists(), "askpass file was not cleaned up after drop");
}
