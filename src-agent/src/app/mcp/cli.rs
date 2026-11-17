//! Argument parsing for `koma mcp …`. Parse only: running lives in
//! `app::runtime::manage::mcp_cli`.

/// Where `add`, `add-json` and `remove` act: the user's `config.json` or the
/// `.mcp.json` of the current directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum McpScope {
    #[default]
    User,
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpCli {
    List {
        json: bool,
    },
    Get {
        name: String,
    },
    Add {
        name: String,
        scope: McpScope,
        transport: Option<String>,
        env: Vec<String>,
        headers: Vec<String>,
        cwd: Option<String>,
        disabled: bool,
        /// Command and arguments, or a single URL.
        target: Vec<String>,
    },
    AddJson {
        name: String,
        scope: McpScope,
        json: String,
    },
    Remove {
        name: String,
        scope: McpScope,
    },
    Approve {
        name: Option<String>,
        all: bool,
        revoke: bool,
    },
    Usage,
}

pub const USAGE: &str = "\
usage: koma mcp <command>

commands:
  list [--json]                         user servers, then the project's .mcp.json
  get <name>                            one server in Claude Code JSON shape
  add <name> [options] -- <command|url> [args...]
      --scope user|project              user config.json (default) or ./.mcp.json
      --transport|-t stdio|http         default stdio, or http for a URL
      --env|-e KEY=VALUE                repeatable
      --header|-H 'Name: value'         repeatable (http)
      --cwd DIR                         working directory (stdio)
      --disabled                        add without enabling
  add-json <name> '<json>' [--scope user|project]
  remove <name> [--scope user|project]
  approve <name>|--all [--revoke]       allow ./.mcp.json servers to run
";

impl McpCli {
    /// `args` are the tokens after the `mcp` verb. Everything after a literal `--` is the
    /// server command, verbatim, so `-y` and `--local` reach the server.
    pub fn parse(args: &[String]) -> Self {
        let (own, target) = match args.iter().position(|a| a == "--") {
            Some(i) => (&args[..i], args[i + 1..].to_vec()),
            None => (args, Vec::new()),
        };
        let mut positionals: Vec<String> = Vec::new();
        let mut transport = None;
        let mut env = Vec::new();
        let mut headers = Vec::new();
        let mut cwd = None;
        let mut scope = McpScope::User;
        let (mut disabled, mut json, mut all, mut revoke) = (false, false, false, false);

        let mut it = own.iter();
        while let Some(arg) = it.next() {
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
                _ => (arg.as_str(), None),
            };
            let value =
                |it: &mut std::slice::Iter<String>| inline.clone().or_else(|| it.next().cloned());
            match flag {
                "--transport" | "-t" => match value(&mut it) {
                    Some(v) => transport = Some(v),
                    None => return Self::Usage,
                },
                "--env" | "-e" => match value(&mut it) {
                    Some(v) => env.push(v),
                    None => return Self::Usage,
                },
                "--header" | "-H" => match value(&mut it) {
                    Some(v) => headers.push(v),
                    None => return Self::Usage,
                },
                "--cwd" => match value(&mut it) {
                    Some(v) => cwd = Some(v),
                    None => return Self::Usage,
                },
                "--scope" | "-s" => match value(&mut it).as_deref() {
                    Some("user") => scope = McpScope::User,
                    Some("project") => scope = McpScope::Project,
                    _ => return Self::Usage,
                },
                "--disabled" => disabled = true,
                "--json" => json = true,
                "--all" => all = true,
                "--revoke" => revoke = true,
                f if f.starts_with('-') && f.len() > 1 => return Self::Usage,
                _ => positionals.push(arg.clone()),
            }
        }

        let mut pos = positionals.into_iter();
        let verb = pos.next();
        let name = pos.next();
        match (verb.as_deref(), name) {
            (Some("list"), None) => Self::List { json },
            (Some("get"), Some(name)) => Self::Get { name },
            (Some("add"), Some(name)) => {
                let target = if target.is_empty() {
                    pos.collect()
                } else {
                    target
                };
                if target.is_empty() {
                    return Self::Usage;
                }
                Self::Add {
                    name,
                    scope,
                    transport,
                    env,
                    headers,
                    cwd,
                    disabled,
                    target,
                }
            }
            (Some("add-json"), Some(name)) => match pos.next() {
                Some(json) => Self::AddJson { name, scope, json },
                None => Self::Usage,
            },
            (Some("remove"), Some(name)) => Self::Remove { name, scope },
            (Some("approve"), name) if name.is_some() || all => Self::Approve {
                name: if all { None } else { name },
                all,
                revoke,
            },
            _ => Self::Usage,
        }
    }
}
