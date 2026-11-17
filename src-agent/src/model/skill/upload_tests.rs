use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;

use super::upload::*;
use super::{OwnedSkillTarget, MAX_SKILL_ZIP_BYTES};

fn tmp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("koma-upload-{}", uuid::Uuid::new_v4()))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn zip_package(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (path, bytes) in files {
            writer.start_file(*path, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }
    cursor.into_inner()
}

#[test]
fn chunks_in_order_assemble_the_file() {
    let dir = tmp_dir();
    let data: Vec<u8> = (0..30u8).collect();
    let total = data.len() as u64;
    assert!(matches!(
        append_chunk(&dir, "up-1", 0, total, &b64(&data[..10])),
        Ok(Staged::More)
    ));
    assert!(matches!(
        append_chunk(&dir, "up-1", 10, total, &b64(&data[10..20])),
        Ok(Staged::More)
    ));
    let Ok(Staged::Complete(path)) = append_chunk(&dir, "up-1", 20, total, &b64(&data[20..]))
    else {
        panic!("expected a complete upload");
    };
    assert_eq!(path, dir.join("up-1.zip"));
    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert!(!dir.join("up-1.part").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn skipped_chunk_is_out_of_order_and_removes_the_part() {
    let dir = tmp_dir();
    assert!(append_chunk(&dir, "up-2", 0, 30, &b64(&[1; 10])).is_ok());
    let err = append_chunk(&dir, "up-2", 20, 30, &b64(&[1; 10]))
        .err()
        .unwrap();
    assert!(err.contains("out of order"));
    assert!(!dir.join("up-2.part").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn total_must_be_within_the_limit() {
    let dir = tmp_dir();
    let err = append_chunk(&dir, "up-3", 0, MAX_SKILL_ZIP_BYTES + 1, &b64(&[1]))
        .err()
        .unwrap();
    assert!(err.contains("256 MiB"));
    assert!(append_chunk(&dir, "up-3", 0, 0, &b64(&[1])).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn chunk_beyond_total_is_rejected() {
    let dir = tmp_dir();
    let err = append_chunk(&dir, "up-4", 0, 5, &b64(&[1; 10]))
        .err()
        .unwrap();
    assert!(err.contains("does not match"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn upload_ids_are_path_safe() {
    for bad in ["../x", "a/b", "a\\b", "", &"a".repeat(65), "C:x", "a.b"] {
        assert!(!valid_upload_id(bad), "{bad:?} must be rejected");
    }
    assert!(valid_upload_id("install-zip-3-lk2f"));
    let dir = tmp_dir();
    assert!(append_chunk(&dir, "../x", 0, 1, &b64(&[1])).is_err());
    assert!(!dir.exists());
}

#[test]
fn bad_base64_and_oversize_chunks_are_rejected() {
    let dir = tmp_dir();
    let err = append_chunk(&dir, "up-5", 0, 10, "not base64 !!")
        .err()
        .unwrap();
    assert!(err.contains("Invalid upload data"));
    let big = "A".repeat(MAX_CHUNK_B64_BYTES + 4);
    let err = append_chunk(&dir, "up-5", 0, 10, &big).err().unwrap();
    assert!(err.contains("too large"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn staged_zip_check_accepts_only_direct_zip_children() {
    let dir = tmp_dir();
    assert!(is_staged_zip(&dir, &dir.join("abc-1.zip")));
    assert!(!is_staged_zip(&dir, &tmp_dir().join("abc-1.zip")));
    assert!(!is_staged_zip(&dir, &dir.join("nested").join("abc-1.zip")));
    assert!(!is_staged_zip(&dir, &dir.join("abc-1.part")));
    assert!(!is_staged_zip(&dir, &dir.join("..").join("x.zip")));
    assert!(!is_staged_zip(&dir, &dir.join("bad id.zip")));
}

#[test]
fn install_staged_installs_then_deletes_even_on_failure() {
    let tmp = tmp_dir();
    let dir = tmp.join("skill-upload");
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("good-1.zip");
    std::fs::write(
        &good,
        zip_package(&[("SKILL.md", b"---\ndescription: Staged\n---\nBody\n")]),
    )
    .unwrap();
    let installed =
        install_staged(&dir, Some(&tmp), OwnedSkillTarget::Project, "staged", &good).unwrap();
    assert!(installed.exists());
    assert!(!good.exists());

    let bad = dir.join("bad-1.zip");
    std::fs::write(&bad, b"not a zip").unwrap();
    assert!(install_staged(&dir, Some(&tmp), OwnedSkillTarget::Project, "broken", &bad).is_err());
    assert!(!bad.exists());

    let foreign = tmp.join("foreign.zip");
    std::fs::write(&foreign, b"x").unwrap();
    let err =
        install_staged(&dir, Some(&tmp), OwnedSkillTarget::Project, "x", &foreign).unwrap_err();
    assert!(err.to_string().contains("not a staged upload"));
    assert!(foreign.exists());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn sweep_removes_only_files_older_than_the_age() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.part"), b"x").unwrap();
    std::fs::write(dir.join("b.zip"), b"x").unwrap();
    std::fs::write(dir.join("keep.txt"), b"x").unwrap();
    sweep_stale(&dir, Duration::from_secs(3600));
    assert!(dir.join("a.part").exists() && dir.join("b.zip").exists());
    sweep_stale(&dir, Duration::ZERO);
    assert!(!dir.join("a.part").exists() && !dir.join("b.zip").exists());
    assert!(dir.join("keep.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
