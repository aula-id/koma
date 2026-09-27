use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub capture: bool,
    pub windows: bool,
    pub focus: bool,
    pub pointer: bool,
    pub keyboard: bool,
    pub accessibility: bool,
    pub ocr: bool,
    pub floating: bool,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub fn valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.width > 0.0
            && self.height > 0.0
            && (self.x + self.width).is_finite()
            && (self.y + self.height).is_finite()
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.valid()
            && x.is_finite()
            && y.is_finite()
            && x >= self.x
            && y >= self.y
            && x < self.x + self.width
            && y < self.y + self.height
    }
}
/// Maps screenshot pixels to desktop coordinates, including negative origins
/// and independently scaled axes. Crops preserve the original desktop origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub desktop: Rect,
    pub width: u32,
    pub height: u32,
}
impl Transform {
    pub fn map(&self, x: f64, y: f64) -> Result<(f64, f64)> {
        if !self.desktop.valid()
            || self.width == 0
            || self.height == 0
            || !(Rect {
                x: 0.0,
                y: 0.0,
                width: self.width as f64,
                height: self.height as f64,
            })
            .contains(x, y)
        {
            bail!("invalid screenshot coordinates or geometry");
        }
        Ok((
            self.desktop.x + x * self.desktop.width / self.width as f64,
            self.desktop.y + y * self.desktop.height / self.height as f64,
        ))
    }
    #[cfg(any(all(feature = "gui", target_os = "linux"), test))]
    pub fn crop(&self, bounds: Rect) -> Result<Self> {
        if !bounds.valid()
            || bounds.x.fract() != 0.0
            || bounds.y.fract() != 0.0
            || bounds.width.fract() != 0.0
            || bounds.height.fract() != 0.0
            || bounds.x + bounds.width > self.width as f64
            || bounds.y + bounds.height > self.height as f64
        {
            bail!("invalid crop");
        }
        let (x, y) = self.map(bounds.x, bounds.y)?;
        Ok(Self {
            desktop: Rect {
                x,
                y,
                width: bounds.width * self.desktop.width / self.width as f64,
                height: bounds.height * self.desktop.height / self.height as f64,
            },
            width: bounds.width as u32,
            height: bounds.height as u32,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub id: String,
    pub application: String,
    pub title: String,
    pub geometry: Rect,
    pub focused: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub id: String,
    pub source: String,
    pub label: String,
    pub role: String,
    pub bounds: Rect,
    pub enabled: bool,
    pub selected: bool,
    pub focused: bool,
    pub confidence: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub session: String,
    pub generation: String,
    pub window: Window,
    pub transform: Transform,
    pub captured_ms: u64,
    pub elements: Vec<Element>,
    pub accessibility_status: String,
    pub ocr_status: String,
    /// Session artifact path, never image bytes in session snapshots.
    pub image_path: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Move {
        x: f64,
        y: f64,
    },
    Click {
        x: Option<f64>,
        y: Option<f64>,
        element: Option<String>,
        button: Button,
    },
    Type {
        text: String,
    },
    Key {
        keys: Vec<String>,
    },
    Scroll {
        x: f64,
        y: f64,
        delta: i32,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Left,
    Right,
    Double,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Windows,
    InspectWindow {
        window: String,
    },
    Select {
        window: String,
        generation: String,
    },
    Observe {
        crop: Option<Rect>,
    },
    Act {
        observation: String,
        actions: Vec<Action>,
        #[serde(default = "yes")]
        observe: bool,
    },
}
fn yes() -> bool {
    true
}
impl Operation {
    pub fn mutates(&self) -> bool {
        matches!(self, Self::Select { .. } | Self::Act { .. })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub session: String,
    pub generation: String,
    pub operation: Operation,
    pub observation: Option<Observation>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub id: String,
    pub session: String,
    pub generation: String,
    pub completed: usize,
    pub uncertain: bool,
    pub error: Option<String>,
    pub windows: Vec<Window>,
    pub capabilities: Option<Capabilities>,
    pub observation: Option<Observation>,
    /// One-shot IPC only. Ingest removes bytes before projecting the status.
    #[serde(with = "png_wire")]
    pub png: Vec<u8>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub session: String,
    pub desktop: String,
    pub generation: String,
    pub enabled: bool,
    pub paused: bool,
    pub busy: bool,
    pub capabilities: Capabilities,
    pub windows: Vec<Window>,
    pub observation: Option<Observation>,
    pub message: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Control {
    Register { desktop: String },
    Enable { capabilities: Capabilities },
    ListWindows,
    InspectWindow { window: String },
    Pause,
    Resume,
    Stop,
    Result(Box<Reply>),
}

// JSON arrays would expand a 20 MiB PNG beyond the IPC frame limit. Base64
// keeps one-shot image payloads bounded while preserving the exact PNG bytes.
mod png_wire {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() > (20_usize * 1024 * 1024).div_ceil(3) * 4 {
            return Err(serde::de::Error::custom("computer PNG exceeds 20 MiB"));
        }
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(serde::de::Error::custom)
    }
}
#[cfg(test)]
mod wire_tests {
    use super::*;
    #[test]
    fn one_shot_png_roundtrip_is_base64_and_byte_identical() {
        let control = Control::Result(Box::new(Reply {
            png: vec![1, 2, 3],
            ..Default::default()
        }));
        let wire = serde_json::to_string(&control).unwrap();
        assert!(wire.contains("AQID"));
        assert_eq!(serde_json::from_str::<Control>(&wire).unwrap(), control);
        assert!(!serde_json::to_string(&Status::default())
            .unwrap()
            .contains("png"));
    }
}
