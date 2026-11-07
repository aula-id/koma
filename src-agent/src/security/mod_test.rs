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
fn a_running_docker_daemon_wins() {
    let seen = RuntimeSeen {
        docker_cli: true,
        docker_daemon: true,
        colima: true,
        podman_cli: true,
        podman_engine: true,
        kubernetes: true,
    };
    assert_eq!(runtime_choice(seen), RuntimeChoice::Ready);
}

#[test]
fn colima_is_used_before_podman_or_kubernetes() {
    let seen = RuntimeSeen {
        docker_cli: false,
        docker_daemon: false,
        colima: true,
        podman_cli: true,
        podman_engine: true,
        kubernetes: true,
    };
    assert_eq!(
        runtime_choice(seen),
        RuntimeChoice::StartColima { need_client: true }
    );
    let with_client = RuntimeSeen {
        docker_cli: true,
        ..seen
    };
    assert_eq!(
        runtime_choice(with_client),
        RuntimeChoice::StartColima { need_client: false }
    );
}

#[test]
fn podman_is_used_when_colima_is_absent() {
    let seen = RuntimeSeen {
        docker_cli: true,
        docker_daemon: false,
        colima: false,
        podman_cli: true,
        podman_engine: false,
        kubernetes: true,
    };
    assert_eq!(
        runtime_choice(seen),
        RuntimeChoice::UsePodman {
            need_client: false,
            need_machine: true
        }
    );
    let running = RuntimeSeen {
        podman_engine: true,
        ..seen
    };
    assert_eq!(
        runtime_choice(running),
        RuntimeChoice::UsePodman {
            need_client: false,
            need_machine: false
        }
    );
}

#[test]
fn kubernetes_alone_is_not_a_local_engine() {
    let seen = RuntimeSeen {
        docker_cli: false,
        docker_daemon: false,
        colima: false,
        podman_cli: false,
        podman_engine: false,
        kubernetes: true,
    };
    assert_eq!(runtime_choice(seen), RuntimeChoice::KubernetesOnly);
}

#[test]
fn a_stopped_docker_desktop_is_not_an_install_prompt() {
    let seen = RuntimeSeen {
        docker_cli: true,
        docker_daemon: false,
        colima: false,
        podman_cli: false,
        podman_engine: false,
        kubernetes: false,
    };
    assert_eq!(runtime_choice(seen), RuntimeChoice::DockerStopped);
    let missing = RuntimeSeen {
        docker_cli: false,
        ..seen
    };
    assert_eq!(runtime_choice(missing), RuntimeChoice::InstallDocker);
}

#[test]
fn the_colima_client_is_the_docker_formula_not_the_cask() {
    assert_eq!(
        docker_client_argv("macos", "brew"),
        Some(vec!["brew", "install", "docker"])
    );
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
