//! Built-in framework adapters using documented JSON/JUnit interfaces.
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
};
#[derive(Deserialize)]
struct Recipe {
    kind: String,
    command: String,
    prefix: Vec<String>,
    args: Vec<String>,
    discover: bool,
    items: Vec<Value>,
}
fn emit(value: Value) {
    println!("KOMA_TEST {value}");
}
fn read(path: &Path) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    std::fs::File::open(path)?
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut data)?;
    anyhow::ensure!(data.len() <= 32 * 1024 * 1024, "Test report exceeds 32 MiB");
    Ok(data)
}
fn execute(recipe: &Recipe, args: Vec<String>, capture: bool) -> Result<(bool, String)> {
    let mut cmd = Command::new(&recipe.command);
    cmd.args(&recipe.prefix)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .stdout(if capture {
            Stdio::piped()
        } else {
            Stdio::inherit()
        });
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn().context("Start project test framework")?;
    let mut bytes = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let result = stdout.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes);
        if result.is_err() || bytes.len() > 32 * 1024 * 1024 {
            let _ = child.kill();
            let _ = child.wait();
            result?;
            anyhow::bail!("Test listing exceeds 32 MiB");
        }
    }
    Ok((child.wait()?.success(), String::from_utf8(bytes)?))
}
fn js_records(report: &Value) -> Result<()> {
    for suite in report["testResults"]
        .as_array()
        .context("Invalid framework JSON report")?
    {
        let file = suite["name"].as_str().unwrap_or("");
        for test in suite["assertionResults"].as_array().into_iter().flatten() {
            let name = test["fullName"]
                .as_str()
                .or_else(|| test["title"].as_str())
                .unwrap_or("");
            let status = match test["status"].as_str() {
                Some("passed") => "passed",
                Some("failed") => "failed",
                _ => "skipped",
            };
            emit(
                json!({"id":format!("{file}::{name}"),"file":file,"label":name,"selector":name,"status":status,"durationMs":test["duration"],"line":test["location"]["line"],"message":test["failureMessages"].as_array().map(|a|a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"))}),
            );
        }
    }
    Ok(())
}
fn junit(bytes: &[u8], kind: &str, mut emit: impl FnMut(Value)) -> Result<()> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut current: Option<Value> = None;
    let mut detail = false;
    let mut depth = 0usize;
    let mut suite = false;
    loop {
        let event = reader.read_event()?;
        let empty = matches!(event, Event::Empty(_));
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                if depth == 0 {
                    anyhow::ensure!(
                        matches!(e.name().as_ref(), b"testsuite" | b"testsuites"),
                        "Invalid JUnit report root"
                    );
                    anyhow::ensure!(!suite, "Multiple JUnit document roots");
                    suite = true;
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).context("Invalid JUnit nesting")?;
            }
            Event::Eof => anyhow::ensure!(suite && depth == 0, "Incomplete JUnit report"),
            _ => {}
        }
        match event {
            Event::DocType(_) => anyhow::bail!("DTD is not supported in test reports"),
            Event::Start(ref e) | Event::Empty(ref e) if e.name().as_ref() == b"testcase" => {
                let mut attrs = std::collections::BTreeMap::new();
                for attr in e.attributes() {
                    let attr = attr?;
                    attrs.insert(
                        String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
                        attr.decode_and_unescape_value(reader.decoder())?
                            .into_owned(),
                    );
                }
                let name = attrs.get("name").cloned().unwrap_or_default();
                let class = attrs.get("classname").cloned().unwrap_or_default();
                let id = if kind == "phpunit" {
                    format!("{class}::{name}")
                } else {
                    name.clone()
                };
                current = Some(
                    json!({"id":id,"label":name,"selector":id,"suite":class,"file":attrs.get("file"),"line":attrs.get("line").and_then(|s|s.parse::<u32>().ok()),"durationMs":attrs.get("time").and_then(|s|s.parse::<f64>().ok()).map(|s|s*1000.),"status":"passed","message":""}),
                );
                if empty {
                    if let Some(v) = current.take() {
                        emit(v);
                    }
                }
            }
            Event::Start(ref e) | Event::Empty(ref e)
                if matches!(e.name().as_ref(), b"failure" | b"error" | b"skipped") =>
            {
                if let Some(v) = current.as_mut() {
                    v["status"] = json!(if e.name().as_ref() == b"skipped" {
                        "skipped"
                    } else {
                        "failed"
                    });
                    for attr in e.attributes() {
                        let attr = attr?;
                        if attr.key.as_ref() == b"message" {
                            v["message"] = json!(attr
                                .decode_and_unescape_value(reader.decoder())?
                                .chars()
                                .take(4096)
                                .collect::<String>());
                        }
                    }
                    detail = !empty;
                }
            }
            Event::Text(e) if detail => {
                if let Some(v) = current.as_mut() {
                    let next = format!(
                        "{}{}",
                        v["message"].as_str().unwrap_or(""),
                        e.xml_content()?
                    );
                    v["message"] = json!(next.chars().take(4096).collect::<String>());
                }
            }
            Event::CData(e) if detail => {
                if let Some(v) = current.as_mut() {
                    v["message"] =
                        json!(
                            format!("{}{}", v["message"].as_str().unwrap_or(""), e.decode()?)
                                .chars()
                                .take(4096)
                                .collect::<String>()
                        );
                }
            }
            Event::GeneralRef(e) if detail => {
                let name = e.decode()?;
                let text = if let Some(c) = e.resolve_char_ref()? {
                    c.to_string()
                } else {
                    match name.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => anyhow::bail!("Unsupported entity in test report"),
                    }
                    .into()
                };
                if let Some(v) = current.as_mut() {
                    v["message"] = json!(format!("{}{text}", v["message"].as_str().unwrap_or(""))
                        .chars()
                        .take(4096)
                        .collect::<String>());
                }
            }
            Event::End(e) if e.name().as_ref() == b"testcase" => {
                if let Some(v) = current.take() {
                    emit(v);
                }
                detail = false;
            }
            Event::End(e) if matches!(e.name().as_ref(), b"failure" | b"error" | b"skipped") => {
                detail = false
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(())
}
pub(super) fn main() -> Result<()> {
    let raw = std::env::args()
        .nth(2)
        .context("Missing framework recipe")?;
    anyhow::ensure!(raw.len() <= 256 * 1024, "Test recipe too large");
    let r: Recipe = serde_json::from_str(&raw)?;
    let base = crate::model::store::base_dir()?.join("coding/reports");
    std::fs::create_dir_all(&base)?;
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Scratch(base.join(uuid::Uuid::new_v4().to_string()));
    std::fs::create_dir(&dir.0)?;
    let report = dir.0.join("report");
    if r.kind == "cargo-doc" {
        let selections = if r.items.is_empty() {
            vec![Value::Null]
        } else {
            r.items.clone()
        };
        let mut failed = false;
        for item in selections {
            let mut args = vec!["test".into(), "--doc".into()];
            args.extend(r.args.clone());
            if let Some(name) = item["selector"].as_str() {
                args.push(name.into());
            }
            args.push("--".into());
            args.extend(["--color".into(), "never".into()]);
            if r.discover {
                args.push("--list".into());
            } else if !item.is_null() {
                args.push("--exact".into());
            }
            let (ok, text) = execute(&r, args, true)?;
            failed |= !ok;
            println!("{text}");
            for line in text.lines() {
                if r.discover {
                    if let Some(name) = line.strip_suffix(": test") {
                        emit(json!({"id":name,"label":name,"selector":name,"status":"discovered"}));
                    }
                } else if let Some((name, status)) = line
                    .strip_prefix("test ")
                    .and_then(|s| s.rsplit_once(" ... "))
                {
                    let state = match status {
                        "ok" => "passed",
                        "FAILED" => "failed",
                        v if v.starts_with("ignored") => "skipped",
                        _ => continue,
                    };
                    emit(json!({"id":name,"label":name,"selector":name,"status":state}));
                }
            }
        }
        anyhow::ensure!(!failed, "Cargo doctests failed; inspect Tasks output");
        return Ok(());
    }
    if r.discover {
        let mut args = r.args.clone();
        match r.kind.as_str() {
            "jest" => {
                args.extend(["--listTests".into(), "--json".into()]);
                let (ok, text) = execute(&r, args, true)?;
                anyhow::ensure!(ok, "Jest collection failed: {text}");
                let files: Vec<String> = serde_json::from_str(&text)?;
                for file in files {
                    emit(json!({"id":file,"file":file,"label":file,"status":"discovered"}));
                }
            }
            "vitest" => {
                args.insert(0, "list".into());
                args.push(format!("--json={}", report.display()));
                let (ok, _) = execute(&r, args, false)?;
                anyhow::ensure!(ok, "Vitest collection failed");
                let tests: Value = serde_json::from_slice(&read(&report)?)?;
                for test in tests.as_array().context("Invalid Vitest listing")? {
                    let file = test["file"].as_str().context("Vitest test has no file")?;
                    let name = test["name"].as_str().context("Vitest test has no name")?;
                    emit(
                        json!({"id":format!("{file}::{name}"),"file":file,"label":name,"selector":name,"line":test["location"]["line"],"status":"discovered"}),
                    );
                }
            }
            "phpunit" => {
                args.push("--list-tests".into());
                let (ok, text) = execute(&r, args, true)?;
                anyhow::ensure!(ok, "PHPUnit collection failed: {text}");
                for line in text.lines() {
                    if let Some(name) = line.trim().strip_prefix("- ") {
                        emit(json!({"id":name,"label":name,"selector":name,"status":"discovered"}));
                    }
                }
            }
            "ctest" => {
                args.push("--show-only=json-v1".into());
                let (ok, text) = execute(&r, args, true)?;
                anyhow::ensure!(ok, "CTest collection failed: {text}");
                let listing: Value = serde_json::from_str(&text)?;
                for test in listing["tests"]
                    .as_array()
                    .context("Invalid CTest listing")?
                {
                    let name = test["name"].as_str().context("CTest has no name")?;
                    emit(json!({"id":name,"label":name,"selector":name,"status":"discovered"}));
                }
            }
            _ => anyhow::bail!("Unknown framework"),
        }
        return Ok(());
    }
    let selections = if r.items.is_empty() {
        vec![Value::Null]
    } else {
        r.items.clone()
    };
    let mut failed = false;
    for item in selections {
        let mut args = r.args.clone();
        let name = item["selector"].as_str();
        match r.kind.as_str() {
            "jest" | "vitest" => {
                if r.kind == "vitest" {
                    args.insert(0, "run".into());
                    args.push("--reporter=json".into());
                } else {
                    args.extend(["--runInBand".into(), "--json".into()]);
                }
                args.push(format!("--outputFile={}", report.display()));
                if let Some(name) = name {
                    args.push(format!("--testNamePattern=^{}$", regex::escape(name)));
                }
                if let Some(file) = item["file"].as_str() {
                    if r.kind == "jest" {
                        args.push("--runTestsByPath".into());
                    }
                    args.push(file.into());
                }
            }
            "phpunit" => {
                args.extend(["--log-junit".into(), report.to_string_lossy().into_owned()]);
                if let Some(name) = name {
                    args.extend([
                        "--filter".into(),
                        format!("/^{}$/", regex::escape(name).replace('/', "\\/")),
                    ]);
                }
            }
            "ctest" => {
                args.extend([
                    "--output-on-failure".into(),
                    "--output-junit".into(),
                    report.to_string_lossy().into_owned(),
                ]);
                if let Some(name) = name {
                    args.extend(["-R".into(), format!("^{}$", regex::escape(name))]);
                }
            }
            _ => anyhow::bail!("Unknown framework"),
        }
        if !item.is_null() {
            let mut running = item.clone();
            running["status"] = json!("running");
            emit(running);
        }
        let _ = std::fs::remove_file(&report);
        let (ok, _) = execute(&r, args, false)?;
        failed |= !ok;
        let bytes = read(&report)
            .context("Framework did not produce its structured report; inspect Tasks output")?;
        if matches!(r.kind.as_str(), "jest" | "vitest") {
            js_records(&serde_json::from_slice(&bytes)?)?;
        } else {
            junit(&bytes, &r.kind, emit)?;
        }
        if !item.is_null() && name.is_none() {
            let mut result = item.clone();
            result["status"] = json!(if ok { "passed" } else { "failed" });
            emit(result);
        }
    }
    anyhow::ensure!(!failed, "One or more framework tests failed");
    Ok(())
}

#[cfg(test)]
mod regression {
    use super::*;
    #[test]
    fn junit_preserves_empty_cases_failure_details_and_skips() {
        let mut rows = Vec::new();
        junit(br#"<testsuite><testcase classname="A" name="pass"/><testcase classname="A" name="fail"><failure><![CDATA[actual < expected]]> &amp; &#33;</failure></testcase><testcase classname="B" name="skip"><skipped message="disabled"/></testcase></testsuite>"#, "phpunit", |v| rows.push(v)).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["id"], "A::pass");
        assert_eq!(rows[0]["status"], "passed");
        assert_eq!(rows[1]["message"], "actual < expected & !");
        assert_eq!(rows[1]["status"], "failed");
        assert_eq!(rows[2]["status"], "skipped");
        assert_eq!(rows[2]["message"], "disabled");
    }
    #[test]
    fn junit_rejects_dtd_and_malformed_reports() {
        assert!(junit(b"<testsuite><testcase/>", "ctest", |_| {}).is_err());
        assert!(junit(b"<other/>", "ctest", |_| {}).is_err());
        assert!(js_records(&json!({})).is_err());
        assert!(junit(b"<!DOCTYPE x><testsuite/>", "ctest", |_| {}).is_err());
        assert!(junit(
            b"<testsuite><testcase></wrong></testsuite>",
            "ctest",
            |_| {}
        )
        .is_err());
    }
}
