//! Stable Cargo artifact identities; runs inside the task supervisor's process tree.
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

#[derive(Deserialize)]
struct Recipe {
    command: String,
    args: Vec<String>,
    discover: bool,
    selected: Vec<String>,
    #[serde(default)]
    debug_output: Option<PathBuf>,
}
struct Artifact {
    key: String,
    label: String,
    file: String,
    executable: PathBuf,
}
fn lines(reader: impl Read, mut visit: impl FnMut(&str) -> Result<()>) -> Result<()> {
    let mut reader = BufReader::new(reader);
    loop {
        let mut line = String::new();
        let count = reader
            .by_ref()
            .take(8 * 1024 * 1024 + 1)
            .read_line(&mut line)?;
        if count == 0 {
            break;
        }
        anyhow::ensure!(
            count <= 8 * 1024 * 1024,
            "Test process emitted an oversized line"
        );
        visit(line.trim_end_matches(['\r', '\n']))?;
    }
    Ok(())
}
fn record(a: &Artifact, name: &str, status: &str) {
    println!(
        "KOMA_TEST {}",
        json!({"id":format!("{}::{name}",a.key),"label":name,"selector":name,"suite":a.label,"file":a.file,"executable":a.executable,"status":status})
    );
}
fn spawn(command: &mut Command) -> std::io::Result<Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn()
}
pub(super) fn main() -> Result<()> {
    let raw = std::env::args()
        .nth(2)
        .context("Missing Cargo test recipe")?;
    anyhow::ensure!(raw.len() <= 256 * 1024, "Cargo recipe is too large");
    let recipe: Recipe = serde_json::from_str(&raw)?;
    let root = std::env::current_dir()?.canonicalize()?;
    let mut build = Command::new(&recipe.command);
    build
        .args(["test", "--no-run", "--message-format=json"])
        .args(&recipe.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .stdin(Stdio::null());
    let mut child = spawn(&mut build).context("Build Cargo test binaries")?;
    let mut artifacts = BTreeMap::new();
    let collected = lines(child.stdout.take().unwrap(), |line| {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if value["reason"] == "compiler-message" {
                if let Some(message) = value["message"]["rendered"].as_str() {
                    print!("{message}");
                }
            } else if value["reason"] == "compiler-artifact" && value["profile"]["test"] == true {
                if let Some(binary) = value["executable"].as_str() {
                    anyhow::ensure!(artifacts.len() < 5000, "Too many Cargo test binaries");
                    let identity = json!([
                        value["package_id"],
                        value["target"]["name"],
                        value["target"]["kind"]
                    ]);
                    let key = format!("{:x}", Sha256::digest(identity.to_string().as_bytes()));
                    let path = Path::new(binary).canonicalize()?;
                    let source = value["target"]["src_path"].as_str().unwrap_or("");
                    let file = Path::new(source)
                        .strip_prefix(&root)
                        .unwrap_or(Path::new(source))
                        .to_string_lossy()
                        .into_owned();
                    artifacts.insert(
                        key.clone(),
                        Artifact {
                            key,
                            label: format!(
                                "{} · {}",
                                value["package_id"].as_str().unwrap_or(""),
                                value["target"]["name"].as_str().unwrap_or("")
                            ),
                            file,
                            executable: path,
                        },
                    );
                }
            }
        } else {
            println!("{line}");
        }
        Ok(())
    });
    if collected.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    collected?;
    anyhow::ensure!(status.success(), "Cargo test build failed ({status})");
    let selected: BTreeSet<_> = recipe.selected.into_iter().collect();
    let mut found = BTreeSet::new();
    let mut failed = false;
    for artifact in artifacts.values() {
        let mut child = spawn(
            Command::new(&artifact.executable)
                .args(["--list", "--format", "terse"])
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .stdin(Stdio::null()),
        )?;
        let mut names = BTreeSet::new();
        let collected = lines(child.stdout.take().unwrap(), |line| {
            if let Some(name) = line.strip_suffix(": test") {
                anyhow::ensure!(names.len() < 5000, "Too many tests in one binary");
                names.insert(name.to_string());
            }
            Ok(())
        });
        if collected.is_err() {
            let _ = child.kill();
        }
        let status = child.wait()?;
        collected?;
        anyhow::ensure!(status.success(), "List tests failed for {}", artifact.label);
        let names: Vec<_> = names
            .into_iter()
            .filter(|name| {
                let id = format!("{}::{name}", artifact.key);
                if selected.is_empty() || selected.contains(&id) {
                    found.insert(id);
                    true
                } else {
                    false
                }
            })
            .collect();
        if let Some(output) = &recipe.debug_output {
            anyhow::ensure!(selected.len() == 1, "Debug exactly one Cargo test");
            if let Some(name) = names.first() {
                super::persistence::atomic_write(
                    output,
                    &serde_json::to_vec(
                        &json!({"program":artifact.executable,"args":["--exact",name,"--nocapture"]}),
                    )?,
                )?;
            }
            continue;
        }
        if recipe.discover {
            for name in names {
                record(artifact, &name, "discovered");
            }
            continue;
        }
        // One exact invocation per selected case; an unfiltered run preserves
        // libtest's own parallel scheduling and avoids rebuilding per test.
        let invocations = if selected.is_empty() {
            vec![names]
        } else {
            names.into_iter().map(|n| vec![n]).collect()
        };
        for names in invocations {
            if names.is_empty() {
                continue;
            }
            let mut run = Command::new(&artifact.executable);
            run.args(["--color", "never"]);
            if !selected.is_empty() {
                run.arg("--exact").arg(&names[0]);
            }
            for name in &names {
                record(artifact, name, "running");
            }
            let mut child = spawn(
                run.stdout(Stdio::piped())
                    .stderr(Stdio::inherit())
                    .stdin(Stdio::null()),
            )?;
            let mut completed = BTreeSet::new();
            let collected = lines(child.stdout.take().unwrap(), |line| {
                println!("{line}");
                if let Some((name, status)) = line
                    .strip_prefix("test ")
                    .and_then(|s| s.rsplit_once(" ... "))
                {
                    let state = match status {
                        "ok" => Some("passed"),
                        "FAILED" => Some("failed"),
                        v if v.starts_with("ignored") => Some("skipped"),
                        _ => None,
                    };
                    if let Some(state) = state {
                        if names.iter().any(|n| n == name) {
                            completed.insert(name.to_string());
                            record(artifact, name, state);
                        }
                    }
                }
                Ok(())
            });
            if collected.is_err() {
                let _ = child.kill();
            }
            let status = child.wait()?;
            collected?;
            failed |= !status.success();
            for name in names {
                if !completed.contains(&name) {
                    record(artifact, &name, "notRun");
                }
            }
        }
    }
    anyhow::ensure!(
        selected.is_subset(&found),
        "Some selected Cargo tests no longer exist; discover again"
    );
    anyhow::ensure!(!failed, "One or more Cargo test binaries failed");
    Ok(())
}
