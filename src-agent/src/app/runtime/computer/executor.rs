//! Shared execution safety boundary, exercised with a deterministic desktop.
use super::*;
use anyhow::{bail, Result};
use std::sync::atomic::{AtomicBool, Ordering};

pub trait Desktop {
    fn windows(&mut self) -> Result<Vec<Window>>;
    fn select(&mut self, id: &str) -> Result<Window>;
    fn inspect(&mut self, id: &str) -> Result<Window>;
    fn capture(&mut self, window: &Window) -> Result<(Transform, Vec<u8>)>;
    fn input(&mut self, action: &Action, transform: &Transform) -> Result<()>;
    fn release(&mut self);
}
pub fn target(action: &Action, observation: &Observation) -> Result<Option<(f64, f64)>> {
    match action {
        Action::Move { x, y } | Action::Scroll { x, y, .. } => Ok(Some((*x, *y))),
        Action::Click { x, y, element, .. } => match (x, y, element) {
            (Some(x), Some(y), None) => Ok(Some((*x, *y))),
            (None, None, Some(id)) => {
                let e = observation
                    .elements
                    .iter()
                    .find(|e| e.id == *id && e.source == "accessibility" && e.enabled)
                    .ok_or_else(|| {
                        anyhow::anyhow!("element is not a visible enabled accessibility target")
                    })?;
                if !e.bounds.valid()
                    || e.bounds.x < 0.0
                    || e.bounds.y < 0.0
                    || e.bounds.x + e.bounds.width > observation.transform.width as f64
                    || e.bounds.y + e.bounds.height > observation.transform.height as f64
                {
                    bail!("invalid element bounds");
                }
                Ok(Some((
                    e.bounds.x + e.bounds.width / 2.0,
                    e.bounds.y + e.bounds.height / 2.0,
                )))
            }
            _ => bail!("click needs either x/y or an observation-scoped element"),
        },
        _ => Ok(None),
    }
}
pub fn validate_actions(obs: &Observation, actions: &[Action], caps: &Capabilities) -> Result<()> {
    if actions.is_empty() || actions.len() > 16 {
        bail!("actions must contain 1..16 steps");
    }
    for (i, action) in actions.iter().enumerate() {
        if let Some((x, y)) = target(action, obs)? {
            if !caps.pointer {
                bail!("pointer unsupported");
            }
            obs.transform.map(x, y)?;
        }
        match action {
            Action::Type { text } if !caps.keyboard || text.len() > 8192 => {
                bail!("keyboard unsupported or text too long")
            }
            Action::Key { keys } if !caps.keyboard || keys.is_empty() || keys.len() > 5 => {
                bail!("invalid key chord or keyboard unsupported")
            }
            // Key events can navigate; conservatively end every chord sequence.
            Action::Key { .. } | Action::Scroll { .. } if i + 1 != actions.len() => {
                bail!("scroll or key navigation must end the sequence")
            }
            Action::Scroll { delta, .. } if delta.unsigned_abs() > 20 || *delta == 0 => {
                bail!("scroll delta must be in -20..20 and nonzero")
            }
            _ => {}
        }
    }
    Ok(())
}
pub fn execute(desktop: &mut dyn Desktop, request: &Request, cancelled: &AtomicBool) -> Reply {
    let mut reply = Reply {
        id: request.id.clone(),
        session: request.session.clone(),
        generation: request.generation.clone(),
        ..Reply::default()
    };
    let result = (|| -> Result<()> {
        if cancelled.load(Ordering::SeqCst) {
            bail!("cancelled");
        }
        let window = match &request.operation {
            Operation::Windows => {
                reply.windows = desktop.windows()?;
                return Ok(());
            }
            Operation::Select { window } => desktop.select(window)?,
            Operation::Observe { crop: Some(_) } => {
                bail!("crop must be served from the persisted observation")
            }
            Operation::Observe { .. } | Operation::Act { .. } => {
                let obs = request
                    .observation
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("select a window first"))?;
                let window = desktop.inspect(&obs.window.id)?;
                if matches!(request.operation, Operation::Act { .. })
                    && (window.geometry != obs.window.geometry || !window.focused)
                {
                    bail!("window moved, resized, closed, or focus changed; observe again");
                }
                window
            }
        };
        if let Operation::Act {
            actions, observe, ..
        } = &request.operation
        {
            let obs = request
                .observation
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing observation"))?;
            for action in actions {
                if cancelled.load(Ordering::SeqCst) {
                    bail!("cancelled; completed inputs were not undone");
                }
                let current = desktop.inspect(&window.id)?;
                if current.geometry != window.geometry || !current.focused {
                    bail!("focus or geometry changed");
                }
                let resolved = if let Action::Click { button, .. } = action {
                    let (x, y) = target(action, obs)?
                        .ok_or_else(|| anyhow::anyhow!("missing click target"))?;
                    Action::Click {
                        x: Some(x),
                        y: Some(y),
                        element: None,
                        button: *button,
                    }
                } else {
                    action.clone()
                };
                reply.uncertain = true;
                desktop.input(&resolved, &obs.transform)?;
                reply.uncertain = false;
                reply.completed += 1;
            }
            if !observe {
                return Ok(());
            }
        }
        if cancelled.load(Ordering::SeqCst) {
            bail!("cancelled");
        }
        let current = desktop.inspect(&window.id)?;
        let (transform, png) = desktop.capture(&current)?;
        if cancelled.load(Ordering::SeqCst) {
            bail!("cancelled; capture discarded");
        }
        reply.png = png;
        reply.observation = Some(Observation {
            id: uuid::Uuid::new_v4().to_string(),
            session: request.session.clone(),
            generation: request.generation.clone(),
            window: current,
            transform,
            captured_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            elements: vec![],
            accessibility_status: "unavailable".into(),
            ocr_status: "unavailable".into(),
            image_path: String::new(),
        });
        Ok(())
    })();
    desktop.release();
    if let Err(e) = result {
        reply.error = Some(e.to_string());
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scaling_and_crop_preserve_negative_origin() {
        let t = Transform {
            desktop: Rect {
                x: -1920.0,
                y: 20.0,
                width: 1920.0,
                height: 1080.0,
            },
            width: 3840,
            height: 2160,
        };
        assert_eq!(t.map(200.0, 100.0).unwrap(), (-1820.0, 70.0));
        let crop = t
            .crop(Rect {
                x: 100.0,
                y: 200.0,
                width: 400.0,
                height: 300.0,
            })
            .unwrap();
        assert_eq!(crop.map(100.0, 50.0).unwrap(), t.map(200.0, 250.0).unwrap());
        assert!(t.map(f64::NAN, 0.0).is_err());
        assert!(t.map(3840.0, 0.0).is_err());
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    struct Fixture {
        captures: usize,
        inputs: usize,
        fail_at: usize,
        released: bool,
        focus: bool,
    }
    impl Fixture {
        fn window(&self) -> Window {
            Window {
                id: "fixture".into(),
                application: "fixture".into(),
                title: "deterministic".into(),
                geometry: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                },
                focused: self.focus,
            }
        }
    }
    impl Desktop for Fixture {
        fn windows(&mut self) -> Result<Vec<Window>> {
            Ok(vec![self.window()])
        }
        fn select(&mut self, _: &str) -> Result<Window> {
            self.focus = true;
            Ok(self.window())
        }
        fn inspect(&mut self, _: &str) -> Result<Window> {
            Ok(self.window())
        }
        fn capture(&mut self, w: &Window) -> Result<(Transform, Vec<u8>)> {
            self.captures += 1;
            Ok((
                Transform {
                    desktop: w.geometry,
                    width: 100,
                    height: 100,
                },
                vec![],
            ))
        }
        fn input(&mut self, _: &Action, _: &Transform) -> Result<()> {
            if self.inputs == self.fail_at {
                bail!("fixture failure");
            }
            self.inputs += 1;
            Ok(())
        }
        fn release(&mut self) {
            self.released = true;
        }
    }
    #[test]
    fn select_inspect_click_type_observe_no_periodic_capture_or_replay() {
        let mut d = Fixture {
            captures: 0,
            inputs: 0,
            fail_at: usize::MAX,
            released: false,
            focus: true,
        };
        let cancel = AtomicBool::new(false);
        let mut r = Request {
            id: "select".into(),
            session: "s".into(),
            generation: "g".into(),
            operation: Operation::Select {
                window: "fixture".into(),
            },
            observation: None,
        };
        let selected = execute(&mut d, &r, &cancel);
        assert_eq!(d.captures, 1);
        r.observation = selected.observation;
        let actions = vec![
            Action::Move { x: 5.0, y: 5.0 },
            Action::Click {
                x: Some(5.0),
                y: Some(5.0),
                element: None,
                button: Button::Left,
            },
            Action::Type {
                text: "hello 世界".into(),
            },
        ];
        r.operation = Operation::Act {
            observation: r.observation.as_ref().unwrap().id.clone(),
            actions: actions.clone(),
            observe: true,
        };
        let result = execute(&mut d, &r, &cancel);
        assert_eq!(result.completed, 3);
        assert_eq!(d.captures, 2);
        assert!(d.released);
        d.fail_at = 4;
        let result = execute(&mut d, &r, &cancel);
        assert_eq!(result.completed, 1);
        assert!(result.uncertain);
        assert_eq!(d.inputs, 4);
        assert_eq!(d.captures, 2);
        cancel.store(true, Ordering::SeqCst);
        execute(&mut d, &r, &cancel);
        assert_eq!(d.inputs, 4);
        cancel.store(false, Ordering::SeqCst);
        d.focus = false;
        let result = execute(&mut d, &r, &cancel);
        assert_eq!(result.completed, 0);
        assert_eq!(d.inputs, 4);
    }
}
