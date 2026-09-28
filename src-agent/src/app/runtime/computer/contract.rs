use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub const OBSERVATION_MAX_EDGE: u32 = 1920;
pub const OBSERVATION_MAX_PIXELS: u64 = 2_073_600;
#[cfg(any(feature = "gui", test))]
pub fn capture_size(width: u32, height: u32, preview: bool) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    let (max_width, max_height, pixels) = if preview {
        (1280.0, 960.0, 1_228_800.0)
    } else {
        (
            f64::from(OBSERVATION_MAX_EDGE),
            f64::from(OBSERVATION_MAX_EDGE),
            OBSERVATION_MAX_PIXELS as f64,
        )
    };
    let scale = 1.0_f64
        .min(max_width / f64::from(width))
        .min(max_height / f64::from(height))
        .min((pixels / (f64::from(width) * f64::from(height))).sqrt());
    (
        (f64::from(width) * scale).floor().max(1.0) as u32,
        (f64::from(height) * scale).floor().max(1.0) as u32,
    )
}

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
    // The wire name is retained for saved sessions. Native pickers return
    // display:* screens plus application windows, which are always view-only.
    pub id: String,
    pub application: String,
    pub title: String,
    pub geometry: Rect,
    pub focused: bool,
    /// Focus identity at observation time for composed desktop sources.
    /// Legacy window observations do not carry it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}
/// Screen IDs are minted by the GUI adapter; other sources are view-only.
pub fn is_screen(id: &str) -> bool {
    id.starts_with("display:") || id.starts_with("portal:screen:")
}
#[derive(Debug)]
pub struct ScreenRequired;
impl std::fmt::Display for ScreenRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Application sharing is assist mode (view-only). Select a screen to control the computer.")
    }
}
impl std::error::Error for ScreenRequired {}

/// A rejected reference never reaches the desktop worker.
#[derive(Debug)]
pub struct ObservationRequired(pub &'static str);
impl std::fmt::Display for ObservationRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for ObservationRequired {}

/// Physical input contention; the adapter records whether this action emitted
/// any events. Earlier completed actions must never be replayed.
#[cfg(any(feature = "gui", test))]
#[derive(Debug)]
pub struct InputBusy {
    pub input_started: bool,
}
#[cfg(any(feature = "gui", test))]
impl std::fmt::Display for InputBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keyboard or mouse is busy; release held keys/buttons before continuing")
    }
}
#[cfg(any(feature = "gui", test))]
impl std::error::Error for InputBusy {}

#[derive(Debug)]
pub struct RegionCaptureUnavailable(pub Rect);
impl std::fmt::Display for RegionCaptureUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "Fresh region capture is unavailable for this source; inspect a saved crop instead",
        )
    }
}
impl std::error::Error for RegionCaptureUnavailable {}

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
        /// Fresh higher-detail capture of a screenshot-relative region. Unlike
        /// crop, this samples desktop pixels again instead of enlarging saved pixels.
        region: Option<Rect>,
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
    #[serde(default)]
    pub input_busy: bool,
    #[serde(default)]
    pub requires_screen: bool,
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

/// GUI-only, ephemeral preview traffic. Never persisted or added to model history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewRequest {
    pub id: String,
    pub session: String,
    pub generation: String,
    pub window: String,
}
#[cfg(feature = "gui")]
#[derive(Debug, Clone, Serialize)]
pub struct PreviewFrame {
    pub request: PreviewRequest,
    pub image: Option<String>,
    pub error: Option<String>,
    pub captured_ms: u64,
    pub width: u32,
    pub height: u32,
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

#[cfg(test)]
mod resolution_tests {
    use super::*;
    #[test]
    fn computer_frames_bound_4k_8k_portrait_and_ultrawide_without_upscaling() {
        for (w, h) in [
            (3840, 2160),
            (7680, 4320),
            (4320, 7680),
            (15360, 4320),
            (5120, 5120),
            (2560, 1600),
            (640, 480),
            (1, 8192),
        ] {
            for preview in [false, true] {
                let (width, height) = capture_size(w, h, preview);
                assert!(width > 0 && height > 0 && width <= w && height <= h);
                assert!(width <= if preview { 1280 } else { 1920 });
                assert!(height <= if preview { 960 } else { 1920 });
                assert!(
                    u64::from(width) * u64::from(height)
                        <= if preview {
                            1_228_800
                        } else {
                            OBSERVATION_MAX_PIXELS
                        }
                );
                assert_eq!(capture_size(width, height, preview), (width, height));
            }
        }
        assert_eq!(capture_size(7680, 4320, false), (1920, 1080));
        assert_eq!(capture_size(7680, 4320, true), (1280, 720));
        assert_eq!(capture_size(640, 480, false), (640, 480));
        assert_eq!(capture_size(0, 480, false), (0, 0));
    }
    #[test]
    fn computer_8k_closeup_maps_negative_origins_and_retina_points() {
        for desktop in [
            Rect {
                x: -7680.0,
                y: -200.0,
                width: 7680.0,
                height: 4320.0,
            },
            Rect {
                x: -3840.0,
                y: 100.0,
                width: 3840.0,
                height: 2160.0,
            },
        ] {
            let overview = Transform {
                desktop,
                width: 1920,
                height: 1080,
            };
            let region = overview
                .crop(Rect {
                    x: 480.0,
                    y: 270.0,
                    width: 480.0,
                    height: 270.0,
                })
                .unwrap()
                .desktop;
            let closeup = Transform {
                desktop: region,
                width: 1920,
                height: 1080,
            };
            assert_eq!(
                closeup.map(960.0, 540.0).unwrap(),
                overview.map(720.0, 405.0).unwrap()
            );
            assert_eq!(
                closeup.map(0.0, 0.0).unwrap(),
                overview.map(480.0, 270.0).unwrap()
            );
            assert!(overview
                .crop(Rect {
                    x: 1900.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0
                })
                .is_err());
            assert!(closeup.map(1920.0, 0.0).is_err());
        }
    }
}
