#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn strip_verbatim_cases() {
    assert_eq!(strip_verbatim(r"\\?\C:\x"), r"C:\x");
    assert_eq!(strip_verbatim(r"\\?\UNC\srv\share\d"), r"\\srv\share\d");
    assert_eq!(strip_verbatim("//?/C:/x"), "C:/x");
    assert_eq!(strip_verbatim(r"\\?\C:"), "C:");
    for same in [
        r"C:\x",
        "/usr/x",
        r"\\?\Volume{1}\x",
        r"\\?\C:x",
        "",
        "é",
        r"\\?\é",
    ] {
        assert_eq!(strip_verbatim(same), same);
    }
}

#[test]
fn win_normalize_cases() {
    assert_eq!(
        win_normalize("C:/Program Files/Git\\bin\\bash.exe"),
        r"C:\Program Files\Git\bin\bash.exe"
    );
    assert_eq!(win_normalize(r"c:\a\\b\"), r"C:\a\b");
    assert_eq!(win_normalize(r"C:\"), r"C:\");
    assert_eq!(win_normalize("C:"), "C:");
    assert_eq!(win_normalize(r"\\srv\share\x"), r"\\srv\share\x");
    assert_eq!(
        win_normalize("C:/Program Files/Git/"),
        r"C:\Program Files\Git"
    );
    assert_eq!(win_normalize(r"\\?\c:\x"), r"C:\x");
}

#[test]
fn win_key_ignores_case_prefix_and_separators() {
    assert_eq!(win_key(r"c:\PROJ\x"), win_key(r"\\?\C:/proj/x"));
}

#[test]
fn win_join_mixes_separators() {
    assert_eq!(
        win_join("C:/Program Files", "Git/bin/bash.exe"),
        r"C:\Program Files\Git\bin\bash.exe"
    );
}

#[test]
fn git_root_from_exec_path_cases() {
    assert_eq!(
        git_root_from_exec_path("C:/Program Files/Git/mingw64/libexec/git-core\r\n").as_deref(),
        Some(r"C:\Program Files\Git")
    );
    assert_eq!(
        git_root_from_exec_path("C:/Git/mingw32/libexec/git-core").as_deref(),
        Some(r"C:\Git")
    );
    assert_eq!(git_root_from_exec_path("C:/x"), None);
    assert_eq!(git_root_from_exec_path("  \r\n"), None);
    assert_eq!(git_root_from_exec_path(""), None);
}

#[test]
fn git_bash_candidates_prefer_bin() {
    assert_eq!(
        git_bash_candidates("C:/Git"),
        [
            r"C:\Git\bin\bash.exe".to_string(),
            r"C:\Git\usr\bin\bash.exe".to_string()
        ]
    );
}

#[test]
fn parse_reg_sz_reads_value_with_spaces() {
    let out = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\GitForWindows\r\n    InstallPath    REG_SZ    C:\\Program Files\\Git\r\n\r\n";
    assert_eq!(parse_reg_sz(out).as_deref(), Some(r"C:\Program Files\Git"));
    assert_eq!(parse_reg_sz("ERROR: not found\r\n"), None);
    assert_eq!(parse_reg_sz("    InstallPath    REG_SZ    \r\n"), None);
    assert_eq!(parse_reg_sz(""), None);
}

#[test]
fn msys_to_win_cases() {
    assert_eq!(msys_to_win("/c/Users/x").as_deref(), Some(r"C:\Users\x"));
    assert_eq!(msys_to_win("/d").as_deref(), Some(r"D:\"));
    for none in ["/tmp/x", "//srv/x", "/cc/x", r"C:\x", "", "/", "/1/x"] {
        assert_eq!(msys_to_win(none), None, "{none}");
    }
}

#[test]
fn shell_kind_and_stem() {
    assert_eq!(
        shell_kind(r"C:\Program Files\Git\bin\BASH.EXE"),
        ShellKind::Bash
    );
    assert_eq!(
        shell_kind(r"C:\Windows\System32\bash.exe"),
        ShellKind::WslBash
    );
    assert_eq!(
        shell_kind("c:/windows/system32/BASH.exe"),
        ShellKind::WslBash
    );
    assert_eq!(shell_kind("/bin/bash"), ShellKind::Bash);
    assert_eq!(
        shell_kind(r"C:\Program Files\PowerShell\7\pwsh.exe"),
        ShellKind::Pwsh
    );
    assert_eq!(
        shell_kind(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
        ShellKind::WindowsPowerShell
    );
    assert_eq!(shell_kind("cmd.exe"), ShellKind::Cmd);
    assert_eq!(shell_kind("/usr/bin/zsh"), ShellKind::Other);
    assert_eq!(shell_stem(r"C:\x\BASH.EXE"), "bash");
    assert_eq!(shell_stem("/bin/zsh"), "zsh");
}

#[test]
fn git_bash_usr_bin_str_cases() {
    assert_eq!(
        git_bash_usr_bin_str(r"C:\Git\bin\bash.exe"),
        r"C:\Git\usr\bin"
    );
    assert_eq!(
        git_bash_usr_bin_str(r"C:\sh\usr\bin\bash.exe"),
        r"C:\sh\usr\bin"
    );
    assert_eq!(
        git_bash_usr_bin_str("C:/Git/BIN/bash.exe"),
        r"C:\Git\usr\bin"
    );
    assert_eq!(git_bash_usr_bin_str("bash.exe"), r"usr\bin");
    assert_eq!(git_bash_usr_bin_str(r"C:\tools\bash.exe"), r"C:\tools");
}

#[test]
fn plain_form_is_kept_only_when_it_exists_and_is_short() {
    assert_eq!(plain_or_verbatim(r"\\?\C:\x", true), r"C:\x");
    assert_eq!(plain_or_verbatim(r"\\?\C:\x", false), r"\\?\C:\x");
    let long = format!(r"\\?\C:\{}", "a".repeat(260));
    assert_eq!(plain_or_verbatim(&long, true), long);
    let ok = format!(r"\\?\C:\{}", "a".repeat(250));
    assert_eq!(plain_or_verbatim(&ok, true), strip_verbatim(&ok));
}

#[test]
#[cfg(not(windows))]
fn display_path_is_identity_off_windows() {
    let p = Path::new(r"\\?\C:\x");
    assert_eq!(display_path(p), p);
    let tmp = std::env::temp_dir();
    assert_eq!(canonical(&tmp).unwrap(), tmp.canonicalize().unwrap());
}
