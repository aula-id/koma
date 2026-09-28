//! Shared execution safety boundary, exercised with a deterministic desktop.
use super::*;
use anyhow::{bail, Result};
#[cfg(any(feature = "gui", test))]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(any(feature = "gui", test))]
pub trait Desktop {
    fn windows(&mut self) -> Result<Vec<Window>>;
    fn select(&mut self, id: &str) -> Result<Window>;
    fn inspect(&mut self, id: &str) -> Result<Window>;
    fn capture(&mut self, window: &Window) -> Result<(Transform, Vec<u8>)>;
    fn capture_region(&mut self, _window: &Window, _region: Rect) -> Result<(Transform, Vec<u8>)> {
        bail!("High-detail region capture is unavailable on this adapter")
    }
    /// A failed preflight sent no input. Adapters also recheck inside input()
    /// because the desktop can change between this check and event injection.
    fn validate_input(&mut self, _action: &Action) -> Result<()> {
        Ok(())
    }
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
            Action::Click { element, .. } if i + 1 != actions.len() => {
                let editable = element
                    .as_ref()
                    .and_then(|id| obs.elements.iter().find(|e| e.id == *id))
                    .is_some_and(|e| {
                        e.source == "accessibility"
                            && e.enabled
                            && matches!(
                                e.role.as_str(),
                                "entry" | "text box" | "editable text" | "combo box"
                            )
                    });
                if !editable {
                    bail!("a click that may navigate must end the sequence; click/type sequences require an accessibility editable-field target");
                }
            }
            Action::Type { text } if !caps.keyboard || text.len() > 8192 => {
                bail!("keyboard unsupported or text too long")
            }
            Action::Type { text }
                if text
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) =>
            {
                bail!("use a final named key action for control characters")
            }
            Action::Key { keys } => {
                if !caps.keyboard {
                    bail!("keyboard input unsupported");
                }
                super::keys::chord(keys)?;
                if i + 1 != actions.len() {
                    bail!("scroll or key navigation must end the sequence");
                }
            }
            Action::Type { text }
                if text.char_indices().any(|(offset, c)| {
                    matches!(c, '\n' | '\r' | '\t')
                        && (i + 1 != actions.len() || offset + c.len_utf8() != text.len())
                }) =>
            {
                bail!("a typed navigation key must be the final character of the sequence; observe before continuing")
            }
            // Scrolling changes visible targets; observe before further input.
            Action::Scroll { .. } if i + 1 != actions.len() => {
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
#[cfg(any(feature = "gui", test))]
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
            Operation::Select { window, .. } => desktop.select(window)?,
            Operation::InspectWindow { window } => desktop.inspect(window)?,
            Operation::Observe { crop: Some(_), .. } => {
                bail!("crop must be served from the persisted observation")
            }
            Operation::Observe { .. } | Operation::Act { .. } => {
                let obs = request
                    .observation
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("select a display first"))?;
                let window = desktop.inspect(&obs.window.id)?;
                if matches!(request.operation, Operation::Act { .. })
                    && (window.geometry != obs.window.geometry
                        || window.title != obs.window.title
                        || !window.focused
                        || window.focus != obs.window.focus)
                {
                    bail!(
                        "window moved, resized, closed, navigated, or focus changed; observe again"
                    );
                }
                window
            }
        };
        if let Operation::Act {
            actions,
            observe,
            observation,
        } = &request.operation
        {
            let source = request
                .observation
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing observation"))?;
            anyhow::ensure!(
                source.id == *observation
                    && source.generation == request.generation
                    && source.session == request.session,
                "stale native observation"
            );
            validate_actions(
                source,
                actions,
                &Capabilities {
                    pointer: true,
                    keyboard: true,
                    ..Default::default()
                },
            )?;
            let obs = request
                .observation
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing observation"))?;
            for action in actions {
                if cancelled.load(Ordering::SeqCst) {
                    bail!("cancelled; completed inputs were not undone");
                }
                let current = desktop.inspect(&window.id)?;
                if current.geometry != window.geometry
                    || current.title != window.title
                    || !current.focused
                    || current.focus != window.focus
                {
                    bail!("focus or geometry changed; observe again");
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
                } else if let Action::Key { keys } = action {
                    Action::Key {
                        keys: super::keys::chord(keys)?,
                    }
                } else {
                    action.clone()
                };
                desktop.validate_input(&resolved)?;
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
        let (transform, png) = if let Operation::Observe {
            region: Some(bounds),
            ..
        } = request.operation
        {
            let source = request
                .observation
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Observe the display first"))?;
            anyhow::ensure!(
                source.window.geometry == current.geometry && source.window.focus == current.focus,
                "Desktop changed; observe again"
            );
            let region = source.transform.crop(bounds)?.desktop;
            desktop.capture_region(&current, region)?
        } else {
            desktop.capture(&current)?
        };
        let after_capture = desktop.inspect(&window.id)?;
        anyhow::ensure!(
            after_capture.geometry == current.geometry && after_capture.focus == current.focus,
            "Desktop changed during capture; observe again"
        );
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
        closed: bool,
        display: bool,
        front: u8,
        reject_key: bool,
    }
    impl Fixture {
        fn window(&self) -> Window {
            Window {
                id: if self.display {
                    "display:fixture"
                } else {
                    "fixture"
                }
                .into(),
                application: "fixture".into(),
                title: "deterministic".into(),
                geometry: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                },
                focused: self.focus,
                focus: self.display.then(|| self.front.to_string()),
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
            anyhow::ensure!(!self.closed, "window closed");
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
                vec![self.front],
            ))
        }
        fn capture_region(&mut self, _: &Window, region: Rect) -> Result<(Transform, Vec<u8>)> {
            self.captures += 1;
            Ok((
                Transform {
                    desktop: region,
                    width: 200,
                    height: 100,
                },
                vec![self.front],
            ))
        }
        fn validate_input(&mut self, action: &Action) -> Result<()> {
            if let Action::Key { keys } = action {
                assert_eq!(
                    keys,
                    &super::super::keys::chord(keys).unwrap(),
                    "native preflight must receive canonical keys"
                );
                if self.reject_key {
                    bail!("Key unavailable in active layout; observe again");
                }
            }
            Ok(())
        }
        fn input(&mut self, action: &Action, _: &Transform) -> Result<()> {
            if self.inputs == self.fail_at {
                bail!("fixture failure");
            }
            self.inputs += 1;
            if self.display && matches!(action, Action::Click { .. }) {
                self.front += 1;
            }
            Ok(())
        }
        fn release(&mut self) {
            self.released = true;
        }
    }
    #[test]
    fn spotlight_aliases_reach_input_and_key_preflight_has_no_uncertain_input() {
        let mut desktop = Fixture {
            captures: 0,
            inputs: 0,
            fail_at: usize::MAX,
            released: false,
            focus: true,
            closed: false,
            display: true,
            front: 1,
            reject_key: false,
        };
        let cancelled = AtomicBool::new(false);
        let mut request = Request {
            id: "select".into(),
            session: "s".into(),
            generation: "g".into(),
            observation: None,
            operation: Operation::Select {
                window: "display:fixture".into(),
                generation: "g".into(),
            },
        };
        request.observation = execute(&mut desktop, &request, &cancelled).observation;
        for keys in [["cmd", "space"], ["Meta", " "]] {
            request.operation = Operation::Act {
                observation: request.observation.as_ref().unwrap().id.clone(),
                actions: vec![Action::Key {
                    keys: keys.map(str::to_string).to_vec(),
                }],
                observe: true,
            };
            let reply = execute(&mut desktop, &request, &cancelled);
            assert!(reply.error.is_none(), "{:?}", reply.error);
            assert_eq!(reply.completed, 1);
            assert!(!reply.uncertain);
            request.observation = reply.observation;
        }
        desktop.reject_key = true;
        request.operation = Operation::Act {
            observation: request.observation.as_ref().unwrap().id.clone(),
            actions: vec![Action::Key {
                keys: vec!["cmd".into(), "space".into()],
            }],
            observe: true,
        };
        let rejected = execute(&mut desktop, &request, &cancelled);
        assert!(rejected.error.unwrap().contains("observe again"));
        assert_eq!(rejected.completed, 0);
        assert!(!rejected.uncertain);
        assert_eq!(desktop.inputs, 2);
        assert_eq!(desktop.captures, 3);
        assert!(desktop.released);

        let mut controller = Controller::default();
        let path = std::env::temp_dir().join(format!("computer-key-{}.lock", uuid::Uuid::new_v4()));
        controller
            .enable(
                1,
                "s",
                "fixture",
                Capabilities {
                    keyboard: true,
                    ..Default::default()
                },
                &path,
            )
            .unwrap();
        let mut obs = request.observation.unwrap();
        obs.generation = controller.status.generation.clone();
        let observation = obs.id.clone();
        controller.status.observation = Some(obs);
        controller.actionable = true;
        assert!(controller
            .begin(
                "bad-name".into(),
                Operation::Act {
                    observation,
                    actions: vec![Action::Key {
                        keys: vec!["cmd".into(), "bogus".into()]
                    }],
                    observe: true
                },
                false
            )
            .is_err());
        assert!(controller.status.enabled && controller.actionable);
        assert!(controller.outbound.is_none());
        drop(controller);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn desktop_click_switches_app_and_returns_composed_frame_without_losing_source() {
        let mut desktop = Fixture {
            captures: 0,
            inputs: 0,
            fail_at: usize::MAX,
            released: false,
            focus: true,
            closed: false,
            display: true,
            front: 1,
            reject_key: false,
        };
        let cancelled = AtomicBool::new(false);
        let mut request = Request {
            id: "desktop".into(),
            session: "s".into(),
            generation: "g".into(),
            operation: Operation::Select {
                window: "display:fixture".into(),
                generation: "g".into(),
            },
            observation: None,
        };
        let selected = execute(&mut desktop, &request, &cancelled);
        assert_eq!(selected.png, vec![1]);
        request.observation = selected.observation;
        request.operation = Operation::Act {
            observation: request.observation.as_ref().unwrap().id.clone(),
            actions: vec![Action::Click {
                x: Some(20.0),
                y: Some(20.0),
                element: None,
                button: Button::Left,
            }],
            observe: true,
        };
        let clicked = execute(&mut desktop, &request, &cancelled);
        assert!(clicked.error.is_none());
        assert_eq!(clicked.completed, 1);
        assert_eq!(clicked.png, vec![2], "observe the newly foregrounded app");
        assert_eq!(
            clicked.observation.as_ref().unwrap().window.id,
            "display:fixture"
        );
        // Reusing a frame from before the app switch cannot target the new focus.
        let stale = execute(&mut desktop, &request, &cancelled);
        assert_eq!(stale.completed, 0);
        assert!(!stale.uncertain);
        assert!(stale.error.unwrap().contains("observe again"));
        assert_eq!(desktop.inputs, 1);
        request.observation = clicked.observation;
        request.operation = Operation::Act {
            observation: request.observation.as_ref().unwrap().id.clone(),
            actions: vec![Action::Type {
                text: "hello 世界".into(),
            }],
            observe: true,
        };
        assert_eq!(execute(&mut desktop, &request, &cancelled).completed, 1);
        assert_eq!(desktop.captures, 3);
        // A fresh close-up is a new capture, but keeps the original display
        // identity and maps local image coordinates into the requested region.
        request.operation = Operation::Observe {
            crop: None,
            region: Some(Rect {
                x: 20.0,
                y: 30.0,
                width: 40.0,
                height: 20.0,
            }),
        };
        let detail = execute(&mut desktop, &request, &cancelled);
        assert!(detail.error.is_none());
        let observation = detail.observation.unwrap();
        assert_eq!(observation.window.geometry.width, 100.0);
        assert_eq!(
            observation.transform.map(100.0, 50.0).unwrap(),
            (40.0, 40.0)
        );
        assert_eq!(desktop.captures, 4);
        desktop.front += 1;
        let stale_detail = execute(&mut desktop, &request, &cancelled);
        assert!(stale_detail.error.unwrap().contains("observe again"));
        assert_eq!(
            desktop.captures, 4,
            "stale region must not capture a different app"
        );
    }

    #[test]
    fn navigation_inside_a_single_action_cannot_send_followup_input() {
        let mut desktop = Fixture {
            captures: 0,
            inputs: 0,
            fail_at: usize::MAX,
            released: false,
            focus: true,
            closed: false,
            display: false,
            front: 1,
            reject_key: false,
        };
        let cancelled = AtomicBool::new(false);
        let mut request = Request {
            id: "select".into(),
            session: "s".into(),
            generation: "g".into(),
            operation: Operation::Select {
                window: "fixture".into(),
                generation: "g".into(),
            },
            observation: None,
        };
        request.observation = execute(&mut desktop, &request, &cancelled).observation;
        for action in [
            Action::Type {
                text: "navigate\nthen type".into(),
            },
            Action::Type {
                text: "tab\tthen type".into(),
            },
            Action::Type {
                text: "escape\u{1b}then type".into(),
            },
            Action::Key {
                keys: vec!["Return".into(), "a".into()],
            },
        ] {
            request.operation = Operation::Act {
                observation: request.observation.as_ref().unwrap().id.clone(),
                actions: vec![action],
                observe: true,
            };
            let result = execute(&mut desktop, &request, &cancelled);
            assert!(result.error.is_some());
            assert_eq!(result.completed, 0);
            assert_eq!(desktop.inputs, 0);
            assert_eq!(desktop.captures, 1);
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
            closed: false,
            display: false,
            front: 1,
            reject_key: false,
        };
        let cancel = AtomicBool::new(false);
        let mut r = Request {
            id: "select".into(),
            session: "s".into(),
            generation: "g".into(),
            operation: Operation::Select {
                window: "fixture".into(),
                generation: "g".into(),
            },
            observation: None,
        };
        let selected = execute(&mut d, &r, &cancel);
        assert_eq!(d.captures, 1);
        r.observation = selected.observation;
        r.observation.as_mut().unwrap().elements.push(Element {
            id: "input".into(),
            source: "accessibility".into(),
            label: "Message".into(),
            role: "entry".into(),
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 20.0,
            },
            enabled: true,
            selected: false,
            focused: false,
            confidence: None,
        });
        let actions = vec![
            Action::Move { x: 5.0, y: 5.0 },
            Action::Click {
                x: None,
                y: None,
                element: Some("input".into()),
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
        d.focus = true;
        r.observation.as_mut().unwrap().window.title = "previous page".into();
        let result = execute(&mut d, &r, &cancel);
        assert_eq!(result.completed, 0);
        assert_eq!(d.inputs, 4);
        assert!(result.error.unwrap().contains("navigated"));
        r.observation.as_mut().unwrap().window.title = d.window().title;
        r.observation.as_mut().unwrap().window.geometry.width = 90.0;
        let result = execute(&mut d, &r, &cancel);
        assert_eq!(result.completed, 0);
        assert!(result.error.unwrap().contains("resized"));
        d.closed = true;
        let result = execute(&mut d, &r, &cancel);
        assert_eq!(result.completed, 0);
        assert!(result.error.unwrap().contains("closed"));
    }
}
