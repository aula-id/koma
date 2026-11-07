use super::*;

#[test]
fn the_release_image_is_koma_safety_at_this_version() {
    let tag = format!("{SAFETY_IMAGE}:{}", env!("CARGO_PKG_VERSION"));
    assert!(tag.starts_with("ghcr.io/aula-id/koma-safety:"));
    assert!(!tag.contains("cyber"));
}

#[test]
fn yes_is_only_y_or_yes() {
    assert!(accepts_yes("y"));
    assert!(accepts_yes("YES"));
    assert!(!accepts_yes("n"));
    assert!(!accepts_yes(""));
}

#[test]
fn missing_requirements_have_a_macos_and_linux_command() {
    assert_eq!(
        requirement_argv("macos", "brew", "python3"),
        Some(vec!["brew", "install", "python@3.12"])
    );
    assert_eq!(
        requirement_argv("macos", "brew", "docker"),
        Some(vec!["brew", "install", "--cask", "docker"])
    );
    assert_eq!(
        requirement_argv("linux", "apt", "docker"),
        Some(vec!["sudo", "apt-get", "install", "-y", "docker.io"])
    );
    assert!(requirement_argv("windows", "brew", "docker").is_none());
}

#[test]
fn the_dockerfile_is_not_shipped_to_the_user() {
    assert!(!ships_to_the_user(std::path::Path::new("image")));
    assert!(!ships_to_the_user(std::path::Path::new("image/Dockerfile")));
    assert!(ships_to_the_user(std::path::Path::new("koma_sec_daemon")));
    assert!(ships_to_the_user(std::path::Path::new("requirements.txt")));
}
