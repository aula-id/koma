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
    /// How long to wait after a completed batch before the final capture.
    /// The fixture returns zero so tests stay fast; real adapters use 200 ms.
    fn post_action_settle(&mut self) -> std::time::Duration {
        std::time::Duration::from_millis(200)
    }
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
    for action in actions {
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
            Action::Type { text }
                if text
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) =>
            {
                bail!("use a named key action for control characters")
            }
            Action::Key { keys } => {
                if !caps.keyboard {
                    bail!("keyboard input unsupported");
                }
                super::keys::chord(keys)?;
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
        let mut window = match &request.operation {
            Operation::Windows => {
                reply.windows = desktop.windows()?;
                return Ok(());
            }
            Operation::Select { window, .. } => {
                if is_screen(window) {
                    desktop.select(window)?
                } else {
                    desktop.inspect(window)?
                }
            }
            Operation::InspectWindow { window } => desktop.inspect(window)?,
            Operation::Observe { crop: Some(_), .. } => {
                bail!("crop must be served from the persisted observation")
            }
            Operation::Observe { .. } | Operation::Act { .. } => {
                let obs = request
                    .observation
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("select a display first"))?;
                if matches!(request.operation, Operation::Act { .. }) && !is_screen(&obs.window.id)
                {
                    return Err(ScreenRequired.into());
                }
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
            for (index, action) in actions.iter().enumerate() {
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
                // Preflight can briefly wait for physical input to clear.
                let ready = desktop.inspect(&window.id)?;
                if ready.geometry != window.geometry
                    || ready.title != window.title
                    || !ready.focused
                    || ready.focus != window.focus
                {
                    bail!("focus or geometry changed during input preflight; observe again");
                }
                if cancelled.load(Ordering::SeqCst) {
                    bail!("cancelled; completed inputs were not undone");
                }
                reply.uncertain = true;
                desktop.input(&resolved, &obs.transform)?;
                reply.uncertain = false;
                reply.completed += 1;
                if index + 1 < actions.len() {
                    // Posted click/key events return before many apps finish
                    // changing focus. Let that transition settle without a
                    // screenshot; cancellation still interrupts the batch.
                    if matches!(action, Action::Click { .. } | Action::Key { .. }) {
                        for _ in 0..10 {
                            if cancelled.load(Ordering::SeqCst) {
                                bail!("cancelled; completed inputs were not undone");
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                    // Refresh native focus metadata after a predicted transition,
                    // without capturing a frame between actions in this batch.
                    let after = desktop.inspect(&window.id)?;
                    if after.geometry != window.geometry || !after.focused {
                        bail!("display geometry or focus unavailable after input; observe again");
                    }
                    window = after;
                }
            }
            if !observe {
                return Ok(());
            }
            // One cancellable pause after the whole batch, including a run of
            // typing. The final capture then sees the painted result. There is
            // still no screenshot between predicted steps.
            if reply.completed > 0 {
                let deadline = std::time::Instant::now() + desktop.post_action_settle();
                while std::time::Instant::now() < deadline {
                    if cancelled.load(Ordering::SeqCst) {
                        bail!("cancelled; completed inputs were not undone");
                    }
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    std::thread::sleep(remaining.min(std::time::Duration::from_millis(10)));
                }
                if cancelled.load(Ordering::SeqCst) {
                    bail!("cancelled; completed inputs were not undone");
                }
            }
        }
        if cancelled.load(Ordering::SeqCst) {
            bail!("cancelled");
        }
        // Input has already completed. Only recapture pixels while an app
        // activation/animation settles; never loop back over injected actions.
        // A region is tied to the old scene and cannot be retargeted this way.
        let region = match request.operation {
            Operation::Observe { region, .. } => region,
            _ => None,
        };
        let attempts = if region.is_some() { 1 } else { 3 };
        let mut attempt = 0;
        let (current, transform, png) = loop {
            if cancelled.load(Ordering::SeqCst) {
                bail!("cancelled; capture discarded");
            }
            let current = desktop.inspect(&window.id)?;
            let (transform, png) = if let Some(bounds) = region {
                let source = request
                    .observation
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Observe the display first"))?;
                anyhow::ensure!(
                    source.window.geometry == current.geometry
                        && source.window.focus == current.focus,
                    "Desktop changed; observe again"
                );
                let region = source.transform.crop(bounds)?.desktop;
                desktop.capture_region(&current, region)?
            } else {
                desktop.capture(&current)?
            };
            if cancelled.load(Ordering::SeqCst) {
                bail!("cancelled; capture discarded");
            }
            let after_capture = desktop.inspect(&window.id)?;
            if cancelled.load(Ordering::SeqCst) {
                bail!("cancelled; capture discarded");
            }
            if after_capture.geometry == current.geometry && after_capture.focus == current.focus {
                break (current, transform, png);
            }
            attempt += 1;
            crate::model::store::append_global_error_log("computer.capture", &format!(
                "session={} request={} scene_changed attempt={attempt}/{attempts} completed={} retry_input=false",
                request.session, request.id, reply.completed,
            ));
            anyhow::ensure!(
                attempt < attempts,
                "Desktop changed during capture; observe again"
            );
            // Bounded, cancellable settling delay on the desktop worker only.
            for _ in 0..5 {
                if cancelled.load(Ordering::SeqCst) {
                    bail!("cancelled; capture discarded");
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
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
        if let Some(busy) = e.downcast_ref::<InputBusy>() {
            reply.input_busy = true;
            reply.uncertain = busy.input_started;
        }
        // Layout rejection is a preflight: no key was sent, so it must not
        // look like a partial injection that stops control.
        if e.to_string().contains("primary keyboard group") {
            reply.uncertain = false;
        }
        reply.requires_screen = e.is::<ScreenRequired>();
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
        busy_at: Option<(usize, bool, bool)>,
        capture_focus_changes: usize,
        cancel_on_capture: Option<std::sync::Arc<AtomicBool>>,
        settles: usize,
    }
    impl Fixture {
        fn window(&self) -> Window {
            Window {
                id: "display:fixture".into(),
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
        fn inspect(&mut self, id: &str) -> Result<Window> {
            anyhow::ensure!(!self.closed, "window closed");
            let mut window = self.window();
            if id.starts_with("app:") {
                window.id = id.into();
            }
            Ok(window)
        }
        fn capture(&mut self, w: &Window) -> Result<(Transform, Vec<u8>)> {
            self.captures += 1;
            if let Some(cancelled) = &self.cancel_on_capture {
                cancelled.store(true, Ordering::SeqCst);
            }
            if self.capture_focus_changes > 0 {
                self.capture_focus_changes -= 1;
                self.front += 1;
            }
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
            if self.busy_at == Some((self.inputs, true, false)) {
                return Err(InputBusy {
                    input_started: false,
                }
                .into());
            }
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
            if let Some((at, false, input_started)) = self.busy_at {
                if at == self.inputs {
                    return Err(InputBusy { input_started }.into());
                }
            }
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
        fn post_action_settle(&mut self) -> std::time::Duration {
            self.settles += 1;
            std::time::Duration::ZERO
        }
    }
    #[test]
    fn predicted_batch_scrolls_keys_and_types_with_one_final_capture() {
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
            busy_at: None,
            capture_focus_changes: 0,
            cancel_on_capture: None,
            settles: 0,
        };
        let cancelled = AtomicBool::new(false);
        let mut request = Request {
            id: "batch".into(),
            session: "s".into(),
            generation: "g".into(),
            observation: None,
            operation: Operation::Select {
                window: "display:fixture".into(),
                generation: "g".into(),
            },
        };
        request.observation = execute(&mut desktop, &request, &cancelled).observation;
        let mut actions = vec![
            Action::Scroll {
                x: 50.0,
                y: 70.0,
                delta: -15
            };
            3
        ];
        actions.extend([
            Action::Click {
                x: Some(10.0),
                y: Some(20.0),
                element: None,
                button: Button::Left,
            },
            Action::Key {
                keys: vec!["Ctrl".into(), "a".into()],
            },
            Action::Type {
                text: "mpos\nsecond line\tvalue".into(),
            },
            Action::Key {
                keys: vec!["Return".into()],
            },
        ]);
        request.operation = Operation::Act {
            observation: request.observation.as_ref().unwrap().id.clone(),
            actions,
            observe: true,
        };
        let reply = execute(&mut desktop, &request, &cancelled);
        assert!(reply.error.is_none(), "{:?}", reply.error);
        assert_eq!(reply.completed, 7);
        assert_eq!(desktop.inputs, 7);
        assert_eq!(desktop.captures, 2, "selection plus one final capture only");
        assert_eq!(
            desktop.settles, 1,
            "one settle after the batch, not per action"
        );
        assert_eq!(
            reply.observation.unwrap().window.focus.as_deref(),
            Some("2")
        );
    }

    #[test]
    fn input_busy_tracks_preflight_races_and_partial_actions_without_replay() {
        for preflight in [false, true] {
            for started in [false, true] {
                if preflight && started {
                    continue;
                }
                for completed in [0, 1] {
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
                        busy_at: Some((completed, preflight, started)),
                        capture_focus_changes: 0,
                        cancel_on_capture: None,
                        settles: 0,
                    };
                    let cancelled = AtomicBool::new(false);
                    let mut request = Request {
                        id: "busy".into(),
                        session: "s".into(),
                        generation: "g".into(),
                        observation: None,
                        operation: Operation::Select {
                            window: "display:fixture".into(),
                            generation: "g".into(),
                        },
                    };
                    request.observation = execute(&mut desktop, &request, &cancelled).observation;
                    request.operation = Operation::Act {
                        observation: request.observation.as_ref().unwrap().id.clone(),
                        actions: vec![
                            Action::Type {
                                text: "mpos".into()
                            };
                            2
                        ],
                        observe: true,
                    };
                    let reply = execute(&mut desktop, &request, &cancelled);
                    assert!(reply.input_busy);
                    assert_eq!(reply.uncertain, started);
                    assert_eq!(reply.completed, completed);
                    assert_eq!(desktop.inputs, completed);
                    assert_eq!(desktop.captures, 1);
                    assert!(desktop.released);
                }
            }
        }
    }

    #[test]
    fn post_click_scene_change_retries_only_capture_and_is_bounded() {
        for (changes, cancel) in [(1, false), (10, false), (1, true)] {
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
                busy_at: None,
                capture_focus_changes: 0,
                cancel_on_capture: None,
                settles: 0,
            };
            let cancelled = std::sync::Arc::new(AtomicBool::new(false));
            let mut request = Request {
                id: "settling".into(),
                session: "s".into(),
                generation: "g".into(),
                observation: None,
                operation: Operation::Select {
                    window: "display:fixture".into(),
                    generation: "g".into(),
                },
            };
            let observation = execute(&mut desktop, &request, &cancelled)
                .observation
                .unwrap();
            request.operation = Operation::Act {
                observation: observation.id.clone(),
                observe: true,
                actions: vec![Action::Click {
                    x: Some(10.0),
                    y: Some(10.0),
                    element: None,
                    button: Button::Left,
                }],
            };
            request.observation = Some(observation);
            desktop.capture_focus_changes = changes;
            desktop.cancel_on_capture = cancel.then(|| cancelled.clone());
            let reply = execute(&mut desktop, &request, &cancelled);
            assert_eq!(reply.completed, 1);
            assert!(!reply.uncertain);
            assert_eq!(
                desktop.inputs, 1,
                "capture retry must never replay the click"
            );
            assert!(desktop.released);
            if cancel {
                assert!(reply.error.unwrap().contains("cancelled"));
                assert!(reply.observation.is_none());
                assert_eq!(desktop.captures, 2, "cancellation prevents recapture");
            } else if changes == 1 {
                assert!(reply.error.is_none(), "{:?}", reply.error);
                assert_eq!(desktop.captures, 3); // Selection + discarded + stable.
                assert_eq!(
                    reply.observation.unwrap().window.focus,
                    Some(desktop.front.to_string())
                );
            } else {
                assert!(reply.error.unwrap().contains("observe again"));
                assert!(reply.observation.is_none());
                assert_eq!(desktop.captures, 4); // Selection + at most three attempts.
                desktop.capture_focus_changes = 0;
                request.operation = Operation::Observe {
                    crop: None,
                    region: None,
                };
                let recovered = execute(&mut desktop, &request, &cancelled);
                assert!(recovered.error.is_none());
                assert!(recovered.observation.is_some());
                assert_eq!(desktop.inputs, 1);
            }
        }
    }
    #[test]
    fn application_assist_blocks_all_input_and_switching_screen_requires_new_observation() {
        let mut desktop = Fixture {
            captures: 0,
            inputs: 0,
            fail_at: usize::MAX,
            released: false,
            focus: false,
            closed: false,
            display: true,
            front: 1,
            reject_key: false,
            busy_at: None,
            capture_focus_changes: 0,
            cancel_on_capture: None,
            settles: 0,
        };
        let cancelled = AtomicBool::new(false);
        let mut request = Request {
            id: "assist".into(),
            session: "s".into(),
            generation: "g".into(),
            observation: None,
            operation: Operation::Select {
                window: "app:fixture".into(),
                generation: "g".into(),
            },
        };
        let selected = execute(&mut desktop, &request, &cancelled);
        assert!(selected.error.is_none());
        assert_eq!(desktop.captures, 1);
        assert!(!desktop.focus, "selecting an application must not focus it");
        let app = selected.observation.unwrap();
        request.observation = Some(app.clone());
        for action in [
            Action::Move { x: 20.0, y: 20.0 },
            Action::Click {
                x: Some(20.0),
                y: Some(20.0),
                element: None,
                button: Button::Left,
            },
            Action::Type {
                text: "no input".into(),
            },
            Action::Key {
                keys: vec!["Return".into()],
            },
            Action::Scroll {
                x: 20.0,
                y: 20.0,
                delta: 1,
            },
        ] {
            request.operation = Operation::Act {
                observation: app.id.clone(),
                actions: vec![action],
                observe: true,
            };
            let refused = execute(&mut desktop, &request, &cancelled);
            assert!(refused.requires_screen);
            assert_eq!(refused.completed, 0);
            assert!(!refused.uncertain);
        }
        assert_eq!(desktop.inputs, 0);
        assert_eq!(desktop.captures, 1);
        request.operation = Operation::Select {
            window: "display:fixture".into(),
            generation: "g".into(),
        };
        let shared = execute(&mut desktop, &request, &cancelled);
        assert!(shared.error.is_none());
        request.observation = shared.observation;
        request.operation = Operation::Act {
            observation: app.id,
            actions: vec![Action::Type {
                text: "hello".into(),
            }],
            observe: true,
        };
        assert!(
            execute(&mut desktop, &request, &cancelled).error.is_some(),
            "an application observation cannot authorize screen input"
        );
        assert_eq!(desktop.inputs, 0);
        if let Operation::Act { observation, .. } = &mut request.operation {
            *observation = request.observation.as_ref().unwrap().id.clone();
        }
        assert_eq!(execute(&mut desktop, &request, &cancelled).completed, 1);
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
            busy_at: None,
            capture_focus_changes: 0,
            cancel_on_capture: None,
            settles: 0,
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
                }
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
            busy_at: None,
            capture_focus_changes: 0,
            cancel_on_capture: None,
            settles: 0,
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
        assert_eq!(
            desktop.settles, 2,
            "each act settles once; observe has not run"
        );
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
        assert_eq!(
            desktop.settles, 2,
            "a region observe does not use the post-action settle"
        );
        desktop.front += 1;
        let stale_detail = execute(&mut desktop, &request, &cancelled);
        assert!(stale_detail.error.unwrap().contains("observe again"));
        assert_eq!(
            desktop.captures, 4,
            "stale region must not capture a different app"
        );
    }

    #[test]
    fn unsupported_control_characters_and_invalid_chords_send_no_input() {
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
            busy_at: None,
            capture_focus_changes: 0,
            cancel_on_capture: None,
            settles: 0,
        };
        let cancelled = AtomicBool::new(false);
        let mut request = Request {
            id: "select".into(),
            session: "s".into(),
            generation: "g".into(),
            operation: Operation::Select {
                window: "display:fixture".into(),
                generation: "g".into(),
            },
            observation: None,
        };
        request.observation = execute(&mut desktop, &request, &cancelled).observation;
        for action in [
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
            busy_at: None,
            capture_focus_changes: 0,
            cancel_on_capture: None,
            settles: 0,
        };
        let cancel = AtomicBool::new(false);
        let mut r = Request {
            id: "select".into(),
            session: "s".into(),
            generation: "g".into(),
            operation: Operation::Select {
                window: "display:fixture".into(),
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
        d.display = true;
        r.observation.as_mut().unwrap().window.focus = Some("1".into());
        let actions = vec![
            Action::Move { x: 5.0, y: 5.0 },
            Action::Click {
                x: Some(10.0),
                y: Some(10.0),
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
        r.observation = result.observation;
        r.operation = Operation::Act {
            observation: r.observation.as_ref().unwrap().id.clone(),
            actions: actions.clone(),
            observe: true,
        };
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

    #[test]
    fn observe_false_skips_settle_and_capture() {
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
            busy_at: None,
            capture_focus_changes: 0,
            cancel_on_capture: None,
            settles: 0,
        };
        let cancelled = AtomicBool::new(false);
        let mut request = Request {
            id: "quiet".into(),
            session: "s".into(),
            generation: "g".into(),
            observation: None,
            operation: Operation::Select {
                window: "display:fixture".into(),
                generation: "g".into(),
            },
        };
        request.observation = execute(&mut desktop, &request, &cancelled).observation;
        request.operation = Operation::Act {
            observation: request.observation.as_ref().unwrap().id.clone(),
            actions: vec![
                Action::Type { text: "one".into() },
                Action::Type { text: "two".into() },
            ],
            observe: false,
        };
        let reply = execute(&mut desktop, &request, &cancelled);
        assert!(reply.error.is_none(), "{:?}", reply.error);
        assert_eq!(reply.completed, 2);
        assert!(reply.observation.is_none());
        assert_eq!(desktop.captures, 1, "selection only; no final frame");
        assert_eq!(desktop.settles, 0);
        request.operation = Operation::Observe {
            crop: None,
            region: None,
        };
        let observed = execute(&mut desktop, &request, &cancelled);
        assert!(observed.observation.is_some());
        assert_eq!(desktop.captures, 2);
        assert_eq!(desktop.settles, 0, "standalone observe stays immediate");
    }
}
