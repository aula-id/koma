//! Strict parsing for the headless CLI. Invalid setup never becomes a silent default.
use super::RunCli;

pub(super) fn parse(args: &[String]) -> RunCli {
    let mut run = RunCli::default();
    if let Err(error) = parse_into(args, &mut run) {
        run.error = Some(error);
    }
    run
}

fn parse_into(args: &[String], run: &mut RunCli) -> Result<(), String> {
    let mut args = args.iter();
    let mut saw_run = false;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "run" if !saw_run => {
                saw_run = true;
                continue;
            }
            "--help" | "-h" => continue,
            "--once" => {
                run.once = true;
                continue;
            }
            "--status" => {
                run.status = true;
                continue;
            }
            "--name"
            | "--prompt"
            | "--prompt-file"
            | "--system"
            | "--system-file"
            | "--timeout"
            | "--session"
            | "--workdir"
            | "--cwd"
            | "--model"
            | "--effort"
            | "--mode"
            | "--security"
            | "--max-tokens"
            | "--short-send"
            | "--context-window-limit"
            | "--context-model-alias"
            | "--extension"
            | "--unload-extension" => {}
            _ => return Err(format!("unknown koma run argument: {flag}")),
        }
        let value = args
            .next()
            .ok_or_else(|| format!("{flag} requires a value"))?;
        if value.starts_with("--") && flag != "--prompt" && flag != "--system" {
            return Err(format!("{flag} requires a value (got {value})"));
        }
        let number = || {
            value
                .parse::<u64>()
                .map_err(|_| format!("{flag} requires a non-negative integer"))
        };
        let toggle =
            || super::parse_on_off(value).ok_or_else(|| format!("{flag} requires on or off"));
        match flag.as_str() {
            "--name" => run.name = Some(value.clone()),
            "--prompt" => run.prompt = Some(value.clone()),
            "--prompt-file" => run.prompt_file = Some(value.clone()),
            "--system" => run.system = Some(value.clone()),
            "--system-file" => run.system_file = Some(value.clone()),
            "--timeout" => run.timeout_sec = number()?.max(1),
            "--session" => run.session = Some(value.clone()),
            "--workdir" | "--cwd" => run.workdir = Some(value.clone()),
            "--model" => run.model = Some(value.clone()),
            "--effort" => run.effort = Some(value.clone()),
            "--mode" => {
                let mode = value.trim().to_ascii_lowercase();
                if !matches!(mode.as_str(), "auto" | "normal" | "plan" | "yolo" | "sdlc") {
                    return Err(format!("unknown mode: {value}"));
                }
                run.mode = Some(mode);
            }
            "--security" => run.security = Some(toggle()?),
            "--max-tokens" => run.max_tokens = Some(number()?.min(1_000_000) as u32),
            "--short-send" => run.short_send = Some(toggle()?),
            "--context-window-limit" => {
                run.context_window_limit =
                    Some(number()?.min(crate::service::context_limits::OPERATING_CEILING))
            }
            "--context-model-alias" => {
                run.context_model_alias = Some(value.trim().chars().take(200).collect())
            }
            "--extension" | "--unload-extension" => {
                if value.trim().is_empty() {
                    return Err(format!("{flag} requires an extension id"));
                }
                let ids = if flag == "--extension" {
                    &mut run.extensions
                } else {
                    &mut run.unload_extensions
                };
                let id = value.trim().to_string();
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            _ => unreachable!(),
        }
    }
    if run.system.is_some() && run.system_file.is_some() {
        return Err("pass only one of --system or --system-file".into());
    }
    if run.mode.as_deref() == Some("yolo") && run.security == Some(false) {
        return Err("--mode yolo requires security on".into());
    }
    if run
        .extensions
        .iter()
        .any(|id| run.unload_extensions.contains(id))
    {
        return Err("an extension cannot be loaded and unloaded in the same command".into());
    }
    Ok(())
}
