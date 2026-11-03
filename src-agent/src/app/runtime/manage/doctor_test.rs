use super::*;

#[test]
fn gui_install_hint_follows_the_distro_family() {
    let cases = [
        ("ID=debian\n", "apt-get install libwebkit2gtk-4.1-0"),
        ("ID=pop\nID_LIKE=\"ubuntu debian\"\n", "apt-get install"),
        ("ID=fedora\n", "dnf install webkit2gtk4.1"),
        (
            "ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n",
            "dnf install",
        ),
        ("ID=arch\n", "pacman -S webkit2gtk-4.1"),
        ("ID=endeavouros\nID_LIKE=arch\n", "pacman -S"),
        (
            "ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n",
            "zypper install",
        ),
    ];
    for (os_release, expected) in cases {
        let hint = gui_install_hint(os_release);
        assert!(hint.contains(expected), "{os_release:?} -> {hint}");
    }
}

#[test]
fn gui_install_hint_names_the_library_when_the_distro_is_unknown() {
    for os_release in ["", "ID=void\n"] {
        let hint = gui_install_hint(os_release);
        assert!(hint.contains("libwebkit2gtk-4.1.so.0"), "{hint}");
        assert!(!hint.contains("sudo"), "{hint}");
    }
}

#[test]
fn window_mode_warns_only_without_gui_conf_outside_debian() {
    use crate::model::linux_window::Mode;
    let (note, warn) = window_mode_note(Some(Mode::Opaque), "ID=fedora\n");
    assert!(note.starts_with("opaque (/etc/koma/gui.conf"), "{note}");
    assert!(!warn);
    let (_, warn) = window_mode_note(Some(Mode::Transparent), "ID=arch\n");
    assert!(!warn, "an explicit choice is respected");
    let (note, warn) = window_mode_note(None, "ID=ubuntu\n");
    assert_eq!(note, "transparent (no gui.conf)");
    assert!(!warn);
    let (_, warn) = window_mode_note(None, "ID=fedora\n");
    assert!(warn);
}
