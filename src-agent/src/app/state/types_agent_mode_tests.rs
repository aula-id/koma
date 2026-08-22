use super::AgentMode;

#[test]
fn cycle_includes_sdlc_when_unarmed() {
    assert_eq!(AgentMode::Plan.cycle(false), AgentMode::Sdlc);
    // SDLC advances to Auto in the pure cycle; phase gating lives at the
    // Shift+Tab call site.
    assert_eq!(AgentMode::Sdlc.cycle(false), AgentMode::Auto);
}

#[test]
fn cycle_includes_sdlc_when_armed() {
    assert_eq!(AgentMode::Plan.cycle(true), AgentMode::Sdlc);
    assert_eq!(AgentMode::Sdlc.cycle(true), AgentMode::Auto);
    assert_eq!(AgentMode::Yolo.cycle(true), AgentMode::Auto);
}

#[test]
fn label_sdlc() {
    assert_eq!(AgentMode::Sdlc.label(), "sdlc");
}
