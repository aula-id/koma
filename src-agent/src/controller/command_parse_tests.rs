#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn parse_clear() {
    assert_eq!(parse("/clear"), Command::Clear);
    assert_eq!(parse("/CLEAR"), Command::Clear);
    assert_eq!(parse("  /clear  "), Command::Clear);
}

#[test]
fn palette_lists_clear() {
    assert!(COMMANDS.iter().any(|(n, _)| *n == "/clear"));
    let matches = palette_matches("/cl");
    assert!(matches.iter().any(|(n, _)| *n == "/clear"));
}

#[test]
fn parse_model_bare() {
    assert_eq!(parse("/model"), Command::Model(String::new()));
    assert_eq!(parse("  /model  "), Command::Model(String::new()));
}

#[test]
fn parse_model_help() {
    assert_eq!(parse("/model help"), Command::Model("help".to_string()));
    assert_eq!(parse("/model ?"), Command::Model("?".to_string()));
}

#[test]
fn parse_model_role() {
    assert_eq!(parse("/model main"), Command::Model("main".to_string()));
    assert_eq!(
        parse("/model awareness"),
        Command::Model("awareness".to_string())
    );
    assert_eq!(
        parse("/model PLANNER"),
        Command::Model("PLANNER".to_string())
    );
}

#[test]
fn parse_model_agent() {
    assert_eq!(parse("/model agent"), Command::Model("agent".to_string()));
    assert_eq!(
        parse("/model agent explore"),
        Command::Model("agent explore".to_string())
    );
}

#[test]
fn parse_attach() {
    assert_eq!(
        parse("/attach .screenshoot/shot.png"),
        Command::Attach(".screenshoot/shot.png".to_string())
    );
    assert_eq!(parse("/attach"), Command::Attach(String::new()));
}

#[test]
fn parse_session_destinations_and_rejects_extra_args() {
    let local_new = NewRequest {
        destination: SessionDestination::Local,
        mode: NewMode::Swap,
    };
    assert_eq!(parse("/new"), Command::New(local_new));
    assert_eq!(parse("/new swap"), Command::New(local_new));
    assert_eq!(
        parse("/new kill"),
        Command::New(NewRequest {
            destination: SessionDestination::Local,
            mode: NewMode::Kill,
        })
    );
    assert_eq!(
        parse("/new remote"),
        Command::New(NewRequest {
            destination: SessionDestination::Remote,
            mode: NewMode::Swap,
        })
    );
    assert!(matches!(parse("/new remote kill"), Command::Unknown(_)));
    assert!(matches!(parse("/new nonsense"), Command::Unknown(_)));

    assert_eq!(parse("/resume"), Command::Resume(SessionDestination::Local));
    assert_eq!(
        parse("/resume remote"),
        Command::Resume(SessionDestination::Remote)
    );
    assert!(matches!(parse("/resume remote extra"), Command::Unknown(_)));
}

#[test]
fn remote_target_is_retained_for_usage_rejection() {
    assert_eq!(parse("/remote"), Command::Remote(String::new()));
    assert_eq!(
        parse("/remote user@example.com"),
        Command::Remote("user@example.com".into())
    );
}

#[test]
fn palette_lists_attach() {
    assert!(COMMANDS.iter().any(|(n, _)| *n == "/attach"));
    let matches = palette_matches("/att");
    assert!(matches.iter().any(|(n, _)| *n == "/attach"));
}
