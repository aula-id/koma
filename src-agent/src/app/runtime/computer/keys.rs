//! One key vocabulary for daemon validation and every native input adapter.
use anyhow::{bail, ensure, Result};

pub fn chord(keys: &[String]) -> Result<Vec<String>> {
    ensure!(
        !keys.is_empty() && keys.len() <= 5,
        "a chord needs 1..5 keys: modifiers followed by one key"
    );
    let keys = keys
        .iter()
        .map(|key| canonical(key))
        .collect::<Result<Vec<_>>>()?;
    let (last, modifiers) = keys
        .split_last()
        .ok_or_else(|| anyhow::anyhow!("empty key chord"))?;
    ensure!(
        !is_modifier(last) && modifiers.iter().all(|key| is_modifier(key)),
        "a chord must contain modifiers followed by one non-modifier key"
    );
    for (index, key) in modifiers.iter().enumerate() {
        ensure!(
            !modifiers[..index].contains(key),
            "duplicate chord modifier: {key}"
        );
    }
    Ok(keys)
}

fn is_modifier(key: &str) -> bool {
    matches!(
        key,
        "Super_L"
            | "Super_R"
            | "Control_L"
            | "Control_R"
            | "Shift_L"
            | "Shift_R"
            | "Alt_L"
            | "Alt_R"
    )
}

fn canonical(key: &str) -> Result<String> {
    ensure!(
        !key.is_empty() && key.len() <= 64 && !key.chars().any(char::is_control),
        "key names must contain 1..64 bytes without control characters"
    );
    // Do not trim: a literal space is a supported key, not an empty name.
    let lower = key.to_ascii_lowercase();
    let named = match lower.as_str() {
        "cmd" | "command" | "meta" | "super" | "win" | "windows" | "command_l" | "cmd_l"
        | "meta_l" | "super_l" => "Super_L",
        "command_r" | "cmd_r" | "meta_r" | "super_r" => "Super_R",
        "ctrl" | "control" | "ctrl_l" | "control_l" => "Control_L",
        "ctrl_r" | "control_r" => "Control_R",
        "shift" | "shift_l" => "Shift_L",
        "shift_r" => "Shift_R",
        "alt" | "option" | "alt_l" | "option_l" => "Alt_L",
        "alt_r" | "option_r" => "Alt_R",
        " " | "space" | "spacebar" => "Space",
        "return" | "enter" => "Return",
        "esc" | "escape" => "Escape",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "del" | "delete" => "Delete",
        "home" => "Home",
        "end" => "End",
        "pageup" | "page_up" | "prior" => "PageUp",
        "pagedown" | "page_down" | "next" => "PageDown",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        _ => {
            if lower.len() == 1 && lower.as_bytes()[0].is_ascii_alphanumeric() {
                return Ok(lower);
            }
            if let Some(number) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                if (1..=24).contains(&number) && lower == format!("f{number}") {
                    // macOS's adapter exposes F1–F12; reject higher keys before
                    // queueing an operation or invalidating its observation.
                    ensure!(
                        !cfg!(target_os = "macos") || number <= 12,
                        "macOS computer key actions support F1 through F12"
                    );
                    return Ok(format!("F{number}"));
                }
            }
            bail!("unsupported computer key name {key:?}; use named keys such as Command, Control, Alt, Shift, Space, Return, Tab, Escape, arrows or F1–F12; use type for text");
        }
    };
    Ok(named.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn computer_spotlight_chords_share_one_adapter_representation() {
        for keys in [
            ["cmd", "space"],
            ["Meta", " "],
            ["COMMAND", "Spacebar"],
            ["Super_L", "Space"],
        ] {
            assert_eq!(
                chord(&keys.map(str::to_string)).unwrap(),
                ["Super_L", "Space"]
            );
        }
        assert_eq!(
            chord(&["option".into(), "SHIFT_R".into(), "ArrowLeft".into()]).unwrap(),
            ["Alt_L", "Shift_R", "Left"]
        );
        assert_eq!(
            chord(&["ctrl".into(), "A".into()]).unwrap(),
            ["Control_L", "a"]
        );
    }
    #[test]
    fn computer_invalid_chords_fail_before_input() {
        for keys in [
            vec![],
            vec!["cmd"],
            vec!["a", "b"],
            vec!["cmd", "Command", "space"],
            vec!["cmd", "unsupported"],
            vec!["Meta", "\t"],
            vec!["F1oops"],
        ] {
            assert!(chord(&keys.into_iter().map(str::to_string).collect::<Vec<_>>()).is_err());
        }
    }
}
