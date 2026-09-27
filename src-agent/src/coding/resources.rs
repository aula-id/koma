//! Text resource edits with complete preflight, a durable inverse journal, and
//! compensating rollback. Multi-file filesystem mutations are not crash-atomic.
use crate::app::runtime::client::file_ops;
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Change {
    path: String,
    fingerprint: String,
    after: Option<String>,
    #[serde(default)]
    format_from: Option<String>,
}
fn path(root: &Path, relative: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        !relative.is_empty() && relative != ".",
        "Cannot modify the workspace root"
    );
    file_ops::resolve_contained_pub(&root.to_string_lossy(), relative, &[root.to_path_buf()])
        .map_err(anyhow::Error::msg)
}
fn bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::File::open(path) {
        Ok(file) => {
            anyhow::ensure!(
                file.metadata()?.is_file(),
                "Resource edits require regular text files"
            );
            let mut bytes = Vec::new();
            file.take(5 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            anyhow::ensure!(
                bytes.len() <= 5 * 1024 * 1024,
                "Resource file exceeds 5 MiB"
            );
            Ok(Some(bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub(super) fn inspect(root: &Path, paths: &[String]) -> Result<Value> {
    anyhow::ensure!(paths.len() <= 100, "Inspect at most 100 resource files");
    let mut values = Vec::new();
    for relative in paths {
        let bytes = bytes(&path(root, relative)?)?;
        let content = bytes
            .as_ref()
            .map(|b| file_ops::decode_resource_text(b))
            .transpose()
            .map_err(anyhow::Error::msg)?;
        values.push(json!({"path":relative,"content":content,"exists":bytes.is_some(),"fingerprint":bytes.as_ref().map(|b|file_ops::fingerprint_bytes(b)).unwrap_or_default()}));
    }
    Ok(json!(values))
}
pub(super) fn apply(root: &Path, changes: &[Change]) -> Result<Value> {
    anyhow::ensure!(
        !changes.is_empty() && changes.len() <= 100,
        "Resource edit must contain 1–100 files"
    );
    let _guard = file_ops::FILE_MUTATION_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("File mutation lock failed"))?;
    let mut originals = BTreeMap::new();
    let mut targets = BTreeMap::new();
    let mut unique = HashSet::new();
    let mut total = 0usize;
    for change in changes {
        let target = path(root, &change.path)?;
        anyhow::ensure!(
            unique.insert(target.clone()),
            "Resource edit aliases the same file more than once"
        );
        let original = bytes(&target)?;
        let hash = original
            .as_ref()
            .map(|b| file_ops::fingerprint_bytes(b))
            .unwrap_or_default();
        anyhow::ensure!(
            hash == change.fingerprint,
            "{} changed on disk; no files were changed",
            change.path
        );
        if let Some(bytes) = &original {
            file_ops::decode_resource_text(bytes).map_err(anyhow::Error::msg)?;
            let meta = target.metadata()?;
            anyhow::ensure!(
                !meta.permissions().readonly(),
                "{} is read-only",
                change.path
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                anyhow::ensure!(
                    meta.nlink() == 1,
                    "Hardlinked resource files are not supported"
                );
            }
        }
        total +=
            original.as_ref().map_or(0, Vec::len) + change.after.as_ref().map_or(0, String::len);
        anyhow::ensure!(total <= 20 * 1024 * 1024, "Resource edit exceeds 20 MiB");
        targets.insert(change.path.clone(), target);
        originals.insert(change.path.clone(), original);
    }
    let mut after = BTreeMap::new();
    for change in changes {
        let format = change.format_from.as_ref().unwrap_or(&change.path);
        let original = originals
            .get(format)
            .context("Format source is not included in resource edit")?;
        let encoded = change
            .after
            .as_ref()
            .map(|content| file_ops::encode_resource_text(original.as_deref(), content))
            .transpose()
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            encoded.as_ref().is_none_or(|b| b.len() <= 5 * 1024 * 1024),
            "Encoded resource exceeds 5 MiB"
        );
        after.insert(change.path.clone(), encoded);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let journal = crate::model::store::base_dir()?
        .join("coding/transactions")
        .join(&id);
    std::fs::create_dir_all(&journal)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&journal, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut records = Vec::new();
    let mut permissions = BTreeMap::new();
    for (i, change) in changes.iter().enumerate() {
        let original = &originals[&change.path];
        if let Some(bytes) = original {
            super::persistence::atomic_write(&journal.join(i.to_string()), bytes)?;
            permissions.insert(
                change.path.clone(),
                targets[&change.path].metadata()?.permissions(),
            );
        }
        let mut record = json!({"path":change.path,"backup":if original.is_some(){Some(i)}else{None},"beforeFingerprint":change.fingerprint,"afterFingerprint":after[&change.path].as_ref().map(|b|file_ops::fingerprint_bytes(b)).unwrap_or_default()});
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = permissions.get(&change.path) {
                record["mode"] = json!(mode.mode());
            }
        }
        records.push(record);
    }
    super::persistence::atomic_write(
        &journal.join("manifest.json"),
        &serde_json::to_vec_pretty(&json!({"root":root,"state":"prepared","files":records}))?,
    )?;
    let mut completed = Vec::new();
    let result = (|| -> Result<()> {
        for change in changes {
            let target = path(root, &change.path)?;
            anyhow::ensure!(
                target == targets[&change.path],
                "Resource path changed during apply"
            );
            let current = bytes(&target)?;
            anyhow::ensure!(
                current == originals[&change.path],
                "{} changed during apply",
                change.path
            );
            completed.push(change.path.clone());
            if let Some(bytes) = &after[&change.path] {
                std::fs::create_dir_all(target.parent().context("No parent directory")?)?;
                super::persistence::atomic_write(&target, bytes)?;
            } else if current.is_some() {
                std::fs::remove_file(&target)?;
            }
            if change.after.is_some() {
                if let Some(mode) = change.format_from.as_ref().and_then(|p| permissions.get(p)) {
                    std::fs::set_permissions(&target, mode.clone())?;
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        for relative in completed.iter().rev() {
            let target = &targets[relative];
            let rollback = (|| -> Result<()> {
                anyhow::ensure!(
                    path(root, relative)? == *target,
                    "Resource path moved during rollback"
                );
                let current = bytes(target)?;
                if current == originals[relative] {
                    return Ok(());
                }
                anyhow::ensure!(
                    current == after[relative],
                    "External edit prevents rollback"
                );
                if let Some(original) = &originals[relative] {
                    super::persistence::atomic_write(target, original)?;
                    if let Some(mode) = permissions.get(relative) {
                        std::fs::set_permissions(target, mode.clone())?;
                    }
                } else if target.exists() {
                    std::fs::remove_file(target)?;
                }
                Ok(())
            })();
            if rollback.is_err() {
                failures.push(relative.clone());
            }
        }
        if failures.is_empty() {
            let _ = std::fs::remove_dir_all(&journal);
            anyhow::bail!("{error:#}; completed changes were rolled back");
        }
        anyhow::bail!(
            "{error:#}; rollback incomplete for {}. Recovery journal: {}",
            failures.join(", "),
            journal.display()
        );
    }
    let files:Vec<Value>=changes.iter().map(|c|json!({"path":c.path,"exists":after[&c.path].is_some(),"fingerprint":after[&c.path].as_ref().map(|b|file_ops::fingerprint_bytes(b)).unwrap_or_default()})).collect();
    super::persistence::atomic_write(
        &journal.join("manifest.json"),
        &serde_json::to_vec_pretty(&json!({"root":root,"state":"applied","files":records}))?,
    ).with_context(||format!("Files were changed, but the transaction journal could not be finalized. Inspect disk before retrying. Recovery journal: {}",journal.display()))?;
    prune(journal.parent().unwrap());
    Ok(json!({"id":id,"files":files}))
}

fn prune(base: &Path) {
    let mut applied: Vec<_> = std::fs::read_dir(base)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let data: Value =
                serde_json::from_slice(&std::fs::read(path.join("manifest.json")).ok()?).ok()?;
            if data["state"] != "applied" {
                return None;
            }
            Some((entry.metadata().ok()?.modified().ok()?, path))
        })
        .collect();
    applied.sort_by_key(|(time, _)| *time);
    let remove = applied.len().saturating_sub(10);
    for (_, path) in applied.into_iter().take(remove) {
        let _ = std::fs::remove_dir_all(path);
    }
}
pub(super) fn undo(root: &Path, id: &str) -> Result<Value> {
    restore(root, id, None)
}
pub(super) fn recover(root: &Path, id: &str, expected: &Value) -> Result<Value> {
    restore(root, id, Some(expected))
}
fn restore(root: &Path, id: &str, expected: Option<&Value>) -> Result<Value> {
    uuid::Uuid::parse_str(id).context("Invalid resource transaction")?;
    let _guard = file_ops::FILE_MUTATION_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("File mutation lock failed"))?;
    let journal = crate::model::store::base_dir()?
        .join("coding/transactions")
        .join(id);
    let mut manifest: Value = serde_json::from_slice(
        &std::fs::read(journal.join("manifest.json"))
            .context("Resource undo is no longer retained")?,
    )?;
    anyhow::ensure!(
        manifest["root"] == json!(root)
            && (manifest["state"] == "applied"
                || expected.is_some()
                    && matches!(manifest["state"].as_str(), Some("prepared" | "undoing"))),
        "Resource transaction does not belong to this workspace or is incomplete"
    );
    let previous_state = manifest["state"].clone();
    let records = manifest["files"]
        .as_array()
        .context("Invalid resource journal")?
        .clone();
    let mut current = BTreeMap::new();
    let mut originals = BTreeMap::new();
    let mut targets = BTreeMap::new();
    for record in &records {
        let relative = record["path"].as_str().context("Invalid resource path")?;
        let target = path(root, relative)?;
        let data = bytes(&target)?;
        let hash = data
            .as_ref()
            .map(|b| file_ops::fingerprint_bytes(b))
            .unwrap_or_default();
        anyhow::ensure!(
            hash == record["afterFingerprint"]
                || expected.is_some() && hash == record["beforeFingerprint"],
            "{relative} has independent edits; recovery will not overwrite them"
        );
        if let Some(expected) = expected {
            anyhow::ensure!(
                expected[relative].as_str() == Some(&hash),
                "{relative} changed after recovery preview"
            );
        }
        let original = record["backup"]
            .as_u64()
            .map(|index| bytes(&journal.join(index.to_string())))
            .transpose()?
            .flatten();
        anyhow::ensure!(
            original
                .as_ref()
                .map(|b| file_ops::fingerprint_bytes(b))
                .unwrap_or_default()
                == record["beforeFingerprint"].as_str().unwrap_or(""),
            "Resource backup is incomplete"
        );
        if let Some(bytes) = &data {
            super::persistence::atomic_write(
                &journal.join(format!("undo-{}", current.len())),
                bytes,
            )?;
        }
        current.insert(relative.to_string(), data);
        originals.insert(relative.to_string(), original);
        targets.insert(relative.to_string(), target);
    }
    manifest["state"] = json!("undoing");
    super::persistence::atomic_write(
        &journal.join("manifest.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    let mut completed = Vec::new();
    let outcome = (|| -> Result<()> {
        for record in &records {
            let relative = record["path"].as_str().unwrap();
            let target = path(root, relative)?;
            anyhow::ensure!(
                target == targets[relative] && bytes(&target)? == current[relative],
                "{relative} changed during undo"
            );
            completed.push(relative.to_string());
            if let Some(bytes) = &originals[relative] {
                std::fs::create_dir_all(target.parent().unwrap())?;
                super::persistence::atomic_write(&target, bytes)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Some(mode) = record["mode"].as_u64() {
                        std::fs::set_permissions(
                            &target,
                            std::fs::Permissions::from_mode(mode as u32),
                        )?;
                    }
                }
            } else if target.exists() {
                std::fs::remove_file(&target)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        let mut failures = Vec::new();
        for relative in completed.iter().rev() {
            let restore = (|| -> Result<()> {
                let target = path(root, relative)?;
                anyhow::ensure!(target == targets[relative], "Path changed");
                let bytes_now = bytes(&target)?;
                if bytes_now == current[relative] {
                    return Ok(());
                }
                anyhow::ensure!(bytes_now == originals[relative], "File changed");
                if let Some(bytes) = &current[relative] {
                    super::persistence::atomic_write(&target, bytes)?;
                } else if target.exists() {
                    std::fs::remove_file(&target)?;
                }
                Ok(())
            })();
            if restore.is_err() {
                failures.push(relative.clone());
            }
        }
        if failures.is_empty() {
            manifest["state"] = previous_state;
            let _ = super::persistence::atomic_write(
                &journal.join("manifest.json"),
                &serde_json::to_vec_pretty(&manifest)?,
            );
            anyhow::bail!("{error:#}; undo rolled back");
        }
        anyhow::bail!(
            "{error:#}; undo rollback incomplete: {}. Recovery journal: {}",
            failures.join(", "),
            journal.display()
        );
    }
    let files:Vec<Value>=records.iter().map(|r|{let relative=r["path"].as_str().unwrap();json!({"path":relative,"exists":originals[relative].is_some(),"fingerprint":r["beforeFingerprint"]})}).collect();
    let _ = std::fs::remove_dir_all(&journal);
    Ok(json!({"files":files}))
}

fn manifest(root: &Path, id: &str) -> Result<(PathBuf, Value)> {
    uuid::Uuid::parse_str(id).context("Invalid resource transaction")?;
    let journal = crate::model::store::base_dir()?
        .join("coding/transactions")
        .join(id);
    let raw = bytes(&journal.join("manifest.json"))?.context("Journal is missing")?;
    let data: Value = serde_json::from_slice(&raw)?;
    anyhow::ensure!(
        data["root"] == json!(root),
        "Journal belongs to another workspace"
    );
    anyhow::ensure!(
        data["files"].as_array().is_some_and(|f| f.len() <= 100),
        "Invalid journal"
    );
    Ok((journal, data))
}
pub(super) fn journals(root: &Path) -> Result<Value> {
    let base = crate::model::store::base_dir()?.join("coding/transactions");
    let mut rows = Vec::new();
    if base.exists() {
        for entry in std::fs::read_dir(base)?.take(1000).flatten() {
            let id = entry.file_name().to_string_lossy().into_owned();
            if let Ok((_, m)) = manifest(root, &id) {
                rows.push(json!({"id":id,"state":m["state"],"files":m["files"].as_array().unwrap().len()}));
            }
        }
    }
    Ok(json!(rows))
}
pub(super) fn recovery_preview(root: &Path, id: &str) -> Result<Value> {
    let _guard = file_ops::FILE_MUTATION_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("File mutation lock failed"))?;
    let (journal, m) = manifest(root, id)?;
    let mut files = Vec::new();
    let mut expected = json!({});
    let mut total = 0;
    for record in m["files"].as_array().unwrap() {
        let relative = record["path"].as_str().context("Invalid resource path")?;
        let current = bytes(&path(root, relative)?)?;
        let original = record["backup"]
            .as_u64()
            .map(|i| bytes(&journal.join(i.to_string())))
            .transpose()?
            .flatten();
        let hash = current
            .as_ref()
            .map(|b| file_ops::fingerprint_bytes(b))
            .unwrap_or_default();
        anyhow::ensure!(
            original
                .as_ref()
                .map(|b| file_ops::fingerprint_bytes(b))
                .unwrap_or_default()
                == record["beforeFingerprint"],
            "Recovery backup is incomplete"
        );
        let decode = |b: &Option<Vec<u8>>| {
            b.as_ref()
                .map(|b| file_ops::decode_resource_text(b))
                .transpose()
                .map_err(anyhow::Error::msg)
        };
        total += current.as_ref().map_or(0, Vec::len) + original.as_ref().map_or(0, Vec::len);
        anyhow::ensure!(total <= 20 * 1024 * 1024, "Recovery preview exceeds 20 MiB");
        expected[relative] = json!(hash);
        files.push(json!({"path":relative,"current":decode(&current)?,"original":decode(&original)?,"conflict":hash!=record["beforeFingerprint"]&&hash!=record["afterFingerprint"]}));
    }
    Ok(json!({"id":id,"state":m["state"],"files":files,"expected":expected}))
}
