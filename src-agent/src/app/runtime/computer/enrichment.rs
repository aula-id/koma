//! Bounded local extraction. Failures never discard a valid screenshot.
use super::*;
use anyhow::{ensure, Result};
use std::{
    io::Read,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
#[cfg(target_os = "linux")]
mod atspi;

pub fn ocr_binary() -> PathBuf {
    let name = if cfg!(windows) {
        "tesseract.exe"
    } else {
        "tesseract"
    };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let sibling = parent.join(name);
            if sibling.is_file() {
                return sibling;
            }
            let bundled = parent.join("ocr").join(name);
            if bundled.is_file() {
                return bundled;
            }
        }
    }
    PathBuf::from(name)
}
pub fn ocr_available() -> bool {
    let binary = ocr_binary();
    binary.is_file()
        || std::env::var_os("PATH")
            .is_some_and(|paths| std::env::split_paths(&paths).any(|p| p.join(&binary).is_file()))
}
pub fn enrich(reply: &mut Reply, cancelled: &AtomicBool) {
    let Some(obs) = reply.observation.as_mut() else {
        return;
    };
    #[cfg(target_os = "linux")]
    match atspi::extract(obs, cancelled) {
        Ok(elements) => {
            obs.elements = elements;
            obs.accessibility_status =
                "AT-SPI selected-window labels, roles, states and bounds; bounded to 256 nodes, 12 levels and 750 ms; values not read".into();
        }
        Err(e) => obs.accessibility_status = format!("unavailable: {e}"),
    }
    if cancelled.load(Ordering::SeqCst) {
        return;
    }
    match ocr(&reply.png, obs, cancelled) {
        Ok(elements) => {
            obs.elements.extend(elements);
            obs.ocr_status =
                "Tesseract local English OCR; at most 256 words / 256 KiB / two seconds; text is not evidence of interactivity".into();
        }
        Err(e) => obs.ocr_status = format!("unavailable: {e}"),
    }
}
fn ocr(png: &[u8], obs: &Observation, cancelled: &AtomicBool) -> Result<Vec<Element>> {
    let dir = std::env::temp_dir().join(format!("koma-ocr-{}", uuid::Uuid::new_v4()));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&dir)?;
    let _cleanup = scopeguard::guard(dir, |p| {
        let _ = std::fs::remove_dir_all(p);
    });
    let input = _cleanup.join("observation.png");
    let output = _cleanup.join("ocr");
    std::fs::write(&input, png)?;
    let binary = ocr_binary();
    let mut cmd = std::process::Command::new(&binary);
    cmd.arg(&input)
        .arg(&output)
        .args(["-l", "eng", "tsv"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(parent) = binary.parent().filter(|p| !p.as_os_str().is_empty()) {
        for relative in [
            "tessdata",
            "../share/tesseract-ocr/5/tessdata",
            "../share/tesseract-ocr/4.00/tessdata",
        ] {
            let data = parent.join(relative);
            if data.join("eng.traineddata").is_file() {
                cmd.env("TESSDATA_PREFIX", data);
                break;
            }
        }
    }
    let mut child = cmd.spawn()?;
    let start = Instant::now();
    let success = loop {
        if let Some(status) = child.try_wait()? {
            break status.success();
        }
        if start.elapsed() > Duration::from_secs(2) || cancelled.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("OCR timed out or cancelled");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    ensure!(
        success,
        "Tesseract failed or English language data is missing"
    );
    let mut text = String::new();
    std::fs::File::open(output.with_extension("tsv"))?
        .take(256 * 1024)
        .read_to_string(&mut text)?;
    Ok(parse_tsv(&text, obs))
}
fn parse_tsv(text: &str, obs: &Observation) -> Vec<Element> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.splitn(12, '\t').collect();
            if fields.len() != 12 || fields[0] != "5" {
                return None;
            }
            let bounds = Rect {
                x: fields[6].parse().ok()?,
                y: fields[7].parse().ok()?,
                width: fields[8].parse().ok()?,
                height: fields[9].parse().ok()?,
            };
            let confidence: f64 = fields[10].parse().ok()?;
            if !bounds.valid()
                || bounds.x < 0.0
                || bounds.y < 0.0
                || bounds.x + bounds.width > obs.transform.width as f64
                || bounds.y + bounds.height > obs.transform.height as f64
                || !confidence.is_finite()
                || !(0.0..=100.0).contains(&confidence)
            {
                return None;
            }
            let label: String = fields[11].chars().take(256).collect();
            if label.trim().is_empty() {
                return None;
            }
            Some(Element {
                id: String::new(),
                source: "ocr".into(),
                label,
                role: "text".into(),
                bounds,
                enabled: false,
                selected: false,
                focused: false,
                confidence: Some(confidence / 100.0),
            })
        })
        .take(256)
        .enumerate()
        .map(|(i, mut e)| {
            e.id = format!("{}:ocr:{i}", obs.id);
            e
        })
        .collect()
}
/// Inspection of a crop reuses the exact persisted observation, without capture.
pub fn crop(request: &Request, bounds: Rect) -> Result<Reply> {
    let source = request
        .observation
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no observation to crop"))?;
    let transform = source.transform.crop(bounds)?;
    let file = std::fs::File::open(&source.image_path)?;
    ensure!(
        file.metadata()?.len() <= 20 * 1024 * 1024,
        "source image too large"
    );
    let mut bytes = vec![];
    file.take(20 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?;
    ensure!(
        image.width() == source.transform.width && image.height() == source.transform.height,
        "persisted observation geometry changed"
    );
    let cropped = image.crop_imm(
        bounds.x as u32,
        bounds.y as u32,
        transform.width,
        transform.height,
    );
    let mut encoded = std::io::Cursor::new(vec![]);
    cropped.write_to(&mut encoded, image::ImageFormat::Png)?;
    let mut obs = source.clone();
    obs.id = uuid::Uuid::new_v4().to_string();
    obs.generation = request.generation.clone();
    obs.transform = transform;
    obs.image_path.clear();
    obs.elements.retain_mut(|e| {
        if !bounds.contains(e.bounds.x, e.bounds.y)
            || e.bounds.x + e.bounds.width > bounds.x + bounds.width
            || e.bounds.y + e.bounds.height > bounds.y + bounds.height
        {
            return false;
        }
        e.bounds.x -= bounds.x;
        e.bounds.y -= bounds.y;
        e.id = format!("{}:{}", obs.id, e.id);
        true
    });
    Ok(Reply {
        id: request.id.clone(),
        session: request.session.clone(),
        generation: request.generation.clone(),
        observation: Some(obs),
        png: encoded.into_inner(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation(path: String) -> Observation {
        Observation {
            id: "original".into(),
            session: "s".into(),
            generation: "g".into(),
            window: Window {
                id: "1:2".into(),
                application: "fixture".into(),
                title: "fixture".into(),
                geometry: Rect {
                    x: -100.0,
                    y: 0.0,
                    width: 4.0,
                    height: 4.0,
                },
                focused: true,
            },
            transform: Transform {
                desktop: Rect {
                    x: -100.0,
                    y: 0.0,
                    width: 4.0,
                    height: 4.0,
                },
                width: 8,
                height: 8,
            },
            captured_ms: 1,
            elements: vec![],
            accessibility_status: "unavailable".into(),
            ocr_status: "unavailable".into(),
            image_path: path,
        }
    }
    #[test]
    fn ocr_is_bounded_noninteractive_and_rejects_bad_bounds() {
        let obs = observation(String::new());
        let text="level\tpage\tblock\tpar\tline\tword\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t1\t2\t3\t4\t95\tHello\n5\t1\t1\t1\t1\t1\t99\t2\t3\t4\t95\tOutside";
        let elements = parse_tsv(text, &obs);
        assert_eq!(elements.len(), 1);
        assert!(!elements[0].enabled);
        assert_eq!(elements[0].source, "ocr");
        assert_eq!(elements[0].confidence, Some(0.95));
    }
    #[test]
    fn crop_reuses_pixels_and_retains_desktop_mapping() {
        let path = std::env::temp_dir().join(format!("koma-crop-{}.png", uuid::Uuid::new_v4()));
        let image = image::RgbImage::from_fn(8, 8, |x, y| image::Rgb([x as u8, y as u8, 42]));
        image.save(&path).unwrap();
        let obs = observation(path.to_string_lossy().into());
        let request = Request {
            id: "crop".into(),
            session: "s".into(),
            generation: "g".into(),
            operation: Operation::Observe { crop: None },
            observation: Some(obs.clone()),
        };
        let result = crop(
            &request,
            Rect {
                x: 2.0,
                y: 2.0,
                width: 4.0,
                height: 4.0,
            },
        )
        .unwrap();
        let decoded = image::load_from_memory(&result.png).unwrap().to_rgb8();
        assert_eq!(decoded.get_pixel(1, 1), image.get_pixel(3, 3));
        assert_eq!(
            result.observation.unwrap().transform.map(1.0, 1.0).unwrap(),
            obs.transform.map(3.0, 3.0).unwrap()
        );
        std::fs::remove_file(path).unwrap();
    }
}
