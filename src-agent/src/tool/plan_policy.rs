//! Host-owned Plan policy. Tool names and server-provided MCP annotations are
//! not proof that an external operation is read-only.

use serde_json::Value;

/// Delegates cannot use the main runtime's planning interceptors: dispatching
/// `checklist` directly, for example, would write the ordinary project TODO.
pub(crate) fn delegated_tool_allowed_in_plan(name: &str) -> bool {
    super::tool_allowed_in_plan(name)
        && !matches!(name, "seqthink" | "checklist" | "plan_enter" | "plan_ready")
}

/// Validate the actual operation, not just its advertised tool name. Every
/// Plan dispatch path (main, deferred, delegated) uses this same policy.
pub(crate) fn plan_tool_call_allowed(name: &str, args: &Value) -> Result<(), String> {
    if !super::tool_allowed_in_plan(name) {
        return Err(format!(
            "plan mode is read-only: {name} is unavailable until the plan is approved"
        ));
    }
    match name {
        "git_operator" => {
            let argv = args
                .get("args")
                .and_then(Value::as_array)
                .and_then(|items| items.iter().map(Value::as_str).collect::<Option<Vec<_>>>())
                .ok_or_else(|| "plan mode is read-only: git args must be strings".to_string())?;
            plan_git_args_allowed(&argv)
        }
        "browser_tabs" | "git_cred" => {
            if args.get("action").and_then(Value::as_str) == Some("list") {
                Ok(())
            } else {
                Err(format!(
                    "plan mode is read-only: {name} permits only action=list"
                ))
            }
        }
        _ => Ok(()),
    }
}

fn plan_git_args_allowed(args: &[&str]) -> Result<(), String> {
    let subcmd = args.first().copied().unwrap_or("");
    let denied = || format!("plan mode is read-only: git {subcmd} permits only read forms");
    if !super::plan_git_subcommand_allowed(subcmd) {
        return Err(denied());
    }
    if subcmd == "branch" {
        return if super::sdlc_assess_branch_is_mutating(args) {
            Err(denied())
        } else {
            Ok(())
        };
    }
    if subcmd == "remote" {
        return if super::sdlc_assess_remote_is_mutating(args) {
            Err(denied())
        } else {
            Ok(())
        };
    }
    // Limit optional switches to supported inspection forms. In particular,
    // output-to-file and external-command switches are not inspection policy.
    for arg in args.iter().skip(1).take_while(|arg| **arg != "--") {
        if !arg.starts_with('-') {
            continue;
        }
        let option = arg.split_once('=').map(|(key, _)| key).unwrap_or(arg);
        if matches!(
            option,
            "--short"
                | "--branch"
                | "--porcelain"
                | "--untracked-files"
                | "--ignored"
                | "--ignore-submodules"
                | "--show-stash"
                | "--ahead-behind"
                | "--oneline"
                | "--graph"
                | "--decorate"
                | "--no-decorate"
                | "--all"
                | "--branches"
                | "--tags"
                | "--remotes"
                | "--reverse"
                | "--date-order"
                | "--topo-order"
                | "--first-parent"
                | "--no-merges"
                | "--merges"
                | "--abbrev-commit"
                | "--no-abbrev-commit"
                | "--pretty"
                | "--format"
                | "--date"
                | "--since"
                | "--until"
                | "--after"
                | "--before"
                | "--author"
                | "--committer"
                | "--grep"
                | "--max-count"
                | "--skip"
                | "--follow"
                | "--stat"
                | "--numstat"
                | "--shortstat"
                | "--name-only"
                | "--name-status"
                | "--summary"
                | "--patch"
                | "--no-patch"
                | "--raw"
                | "--cached"
                | "--staged"
                | "--check"
                | "--quiet"
                | "--exit-code"
                | "--no-color"
                | "--color"
                | "--no-ext-diff"
                | "--no-textconv"
                | "--word-diff"
                | "--word-diff-regex"
                | "--unified"
                | "--relative"
                | "--ignore-space-change"
                | "--ignore-all-space"
                | "--ignore-space-at-eol"
                | "--ignore-blank-lines"
                | "--diff-filter"
                | "--find-renames"
                | "--find-copies"
                | "--binary"
                | "--full-index"
                | "--abbrev"
                | "--verify"
                | "--symbolic"
                | "--symbolic-full-name"
                | "--abbrev-ref"
                | "--show-toplevel"
                | "--show-prefix"
                | "--git-dir"
                | "--git-common-dir"
                | "--is-inside-work-tree"
                | "--is-bare-repository"
                | "--dirty"
                | "--always"
                | "--long"
                | "--contains"
                | "--exact-match"
                | "--match"
                | "--exclude"
                | "--count"
                | "--heads"
                | "--refs"
                | "--get-url"
                | "--stage"
                | "--deleted"
                | "--modified"
                | "--others"
                | "--exclude-standard"
                | "--error-unmatch"
                | "--full-name"
                | "--line-porcelain"
                | "-s"
                | "-b"
                | "-v"
                | "-n"
                | "-p"
                | "-q"
                | "-z"
                | "-w"
                | "-i"
                | "-l"
                | "-a"
                | "-r"
                | "-t"
                | "-u"
                | "-U"
                | "-M"
                | "-C"
                | "-S"
                | "-G"
        ) || arg
            .strip_prefix('-')
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            || ["-n", "-U", "-M", "-C"].iter().any(|prefix| {
                arg.strip_prefix(*prefix)
                    .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            })
        {
            continue;
        }
        return Err(format!(
            "plan mode is read-only: git option {arg} is not an approved inspection option"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plan_checks_git_operations_and_browser_actions() {
        for argv in [
            json!(["status", "--short"]),
            json!(["log", "--oneline", "-5"]),
            json!(["diff", "--stat"]),
            json!(["branch", "--list", "feature/*"]),
            json!(["remote", "get-url", "origin"]),
        ] {
            assert!(plan_tool_call_allowed("git_operator", &json!({"args": argv})).is_ok());
        }
        for argv in [
            json!(["branch", "feature/new"]),
            json!(["branch", "-d", "old"]),
            json!(["remote", "rename", "origin", "upstream"]),
            json!(["commit", "-m", "change"]),
            json!(["diff", "--output=report.patch"]),
            json!(["log", "--unknown-option"]),
            json!(["status", 1]),
        ] {
            assert!(plan_tool_call_allowed("git_operator", &json!({"args": argv})).is_err());
        }
        for name in ["browser_tabs", "git_cred"] {
            assert!(plan_tool_call_allowed(name, &json!({"action":"list"})).is_ok());
            assert!(plan_tool_call_allowed(name, &json!({"action":"select"})).is_err());
            assert!(plan_tool_call_allowed(name, &json!({})).is_err());
        }
    }

    #[test]
    fn plan_denies_untrusted_external_tools_and_delegate_only_interceptors() {
        for name in [
            "mcp__server__read",
            "mcp__server__write",
            "sec_scan",
            "write",
        ] {
            assert!(!super::super::tool_allowed_in_plan(name));
            assert!(!delegated_tool_allowed_in_plan(name));
            assert!(plan_tool_call_allowed(name, &json!({"readOnlyHint":true})).is_err());
        }
        for name in ["checklist", "seqthink", "plan_enter", "plan_ready"] {
            assert!(!delegated_tool_allowed_in_plan(name));
        }
        assert!(delegated_tool_allowed_in_plan("read"));
    }
}
