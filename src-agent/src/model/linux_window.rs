//! Linux GUI window mode, chosen once per machine by install.sh and written to
//! [`CONFIG_PATH`]. WebKitGTK 2.54 leaves undamaged pixels at alpha 0 in an
//! RGBA toplevel on a compositing desktop (#242), so the installer picks an
//! opaque window outside the Debian family. A missing or unreadable file keeps
//! the transparent window every install had before.
//!
//! `koma doctor` reads the rest on Linux only; elsewhere the GUI uses just
//! [`transparent`], which is always true there.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

pub const CONFIG_PATH: &str = "/etc/koma/gui.conf";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Transparent,
    Opaque,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transparent => "transparent",
            Self::Opaque => "opaque",
        }
    }
}

/// `window=` from a `gui.conf` body. The last valid line wins; comments,
/// blank lines and unknown values are ignored.
pub fn parse(text: &str) -> Option<Mode> {
    let mut mode = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "window" {
            continue;
        }
        match value.trim() {
            "transparent" => mode = Some(Mode::Transparent),
            "opaque" => mode = Some(Mode::Opaque),
            _ => {}
        }
    }
    mode
}

/// The mode the installer wrote, if any.
pub fn configured() -> Option<Mode> {
    parse(&std::fs::read_to_string(CONFIG_PATH).ok()?)
}

/// Whether GUI windows are created transparent. Always true off Linux. Read
/// once, so every window in the process agrees.
#[cfg(feature = "gui")]
pub fn transparent() -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    static TRANSPARENT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRANSPARENT.get_or_init(|| configured() != Some(Mode::Opaque))
}

/// `ID` and `ID_LIKE` tokens from an `/etc/os-release` body.
pub fn os_ids(os_release: &str) -> Vec<&str> {
    let mut ids = Vec::new();
    for line in os_release.lines() {
        if let Some(value) = line
            .strip_prefix("ID=")
            .or_else(|| line.strip_prefix("ID_LIKE="))
        {
            ids.extend(value.trim_matches(['"', '\'']).split_whitespace());
        }
    }
    ids
}

/// The family install.sh keeps transparent.
pub fn is_debian_family(os_release: &str) -> bool {
    os_ids(os_release)
        .iter()
        .any(|id| *id == "debian" || *id == "ubuntu")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_the_window_line() {
        assert_eq!(parse("window=opaque\n"), Some(Mode::Opaque));
        assert_eq!(
            parse("# Written by install.sh\n window = transparent \n"),
            Some(Mode::Transparent)
        );
        assert_eq!(
            parse("window=opaque\nwindow=transparent\n"),
            Some(Mode::Transparent)
        );
        assert_eq!(parse("window=opaque\nwindow=glass\n"), Some(Mode::Opaque));
    }

    #[test]
    fn parse_ignores_missing_and_unknown_values() {
        for text in [
            "",
            "# comment\n",
            "window=glass\n",
            "windowopaque\n",
            "other=opaque\n",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn debian_family_follows_id_and_id_like() {
        for os_release in [
            "ID=debian\n",
            "ID=ubuntu\n",
            "ID=pop\nID_LIKE=\"ubuntu debian\"\n",
            "ID=linuxmint\nID_LIKE='ubuntu'\n",
        ] {
            assert!(is_debian_family(os_release), "{os_release:?}");
        }
        for os_release in [
            "",
            "ID=fedora\n",
            "ID=arch\n",
            "ID=endeavouros\nID_LIKE=arch\n",
            "ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n",
            "ID=nixos\n",
        ] {
            assert!(!is_debian_family(os_release), "{os_release:?}");
        }
    }
}
