use super::*;

#[test]
fn the_installer_pulls_the_floating_minimal_image() {
    let tag = safety_image_tag_for("minimal");
    assert_eq!(tag, format!("{SAFETY_IMAGE}:latest"));
    assert!(!tag.contains(env!("CARGO_PKG_VERSION")));
    assert!(!tag.contains("cyber"));
}

#[test]
fn safety_image_tag_is_latest_for_every_composition() {
    let prev = std::env::var_os("KOMA_SEC_IMAGE");
    std::env::remove_var("KOMA_SEC_IMAGE");
    let tag = safety_image_tag();
    assert_eq!(tag, format!("{SAFETY_IMAGE}:latest"));
    assert_eq!(tag, safety_image_tag_for("minimal"));
    assert_eq!(
        safety_image_tag_for("web"),
        format!("{SAFETY_IMAGE}:latest-web")
    );
    match prev {
        Some(v) => std::env::set_var("KOMA_SEC_IMAGE", v),
        None => std::env::remove_var("KOMA_SEC_IMAGE"),
    }
}

#[test]
fn known_compositions_include_minimal_and_domain_shards() {
    let ids: Vec<&str> = list_known_compositions().iter().map(|(id, _)| *id).collect();
    assert_eq!(ids.first().copied(), Some("minimal"));
    for need in [
        "recon",
        "recon-2",
        "recon-ext",
        "recon-src",
        "web",
        "web-ext",
        "web-src",
        "identity",
        "cloud",
        "rev",
        "crypto",
        "pxn",
        "dfir",
        "lab",
        "lab-src",
    ] {
        assert!(ids.contains(&need), "missing {need}");
    }
    assert_eq!(ids.len(), 24);
    for gone in RETIRED_COMPOSITIONS {
        assert!(!ids.contains(gone), "{gone} is retired and must not be offered");
    }
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
        requirement_argv("linux", "apt", "docker"),
        Some(vec!["sudo", "apt-get", "install", "-y", "docker.io"])
    );
    assert!(requirement_argv("windows", "brew", "docker").is_none());
}

#[test]
fn nothing_installed_offers_colima_on_macos_and_the_docker_engine_on_linux() {
    assert_eq!(
        offered_runtime_argv("macos", "brew"),
        Some(vec!["brew", "install", "colima", "docker"])
    );
    assert_eq!(
        offered_runtime_argv("linux", "apt"),
        Some(vec!["sudo", "apt-get", "install", "-y", "docker.io"])
    );
    assert_eq!(
        offered_runtime_argv("linux", "dnf"),
        Some(vec!["sudo", "dnf", "install", "-y", "docker"])
    );
    assert!(offered_runtime_argv("windows", "brew").is_none());
    assert!(offered_runtime_argv("other", "brew").is_none());
}

#[test]
fn the_dockerfile_is_not_shipped_to_the_user() {
    assert!(!ships_to_the_user(std::path::Path::new("image")));
    assert!(!ships_to_the_user(std::path::Path::new("image/Dockerfile")));
    assert!(ships_to_the_user(std::path::Path::new("koma_sec_daemon")));
    assert!(ships_to_the_user(std::path::Path::new("requirements.txt")));
}

#[test]
fn runtime_label_prefers_colima_then_podman_when_docker_is_up() {
    assert_eq!(
        runtime_label(true, true, true, false),
        "docker ready (colima)"
    );
    assert_eq!(
        runtime_label(true, false, true, false),
        "docker ready (podman)"
    );
    assert_eq!(runtime_label(true, false, false, false), "docker ready");
}

#[test]
fn runtime_label_describes_down_and_missing_engines() {
    assert_eq!(
        runtime_label(false, true, false, false),
        "docker down — colima installed"
    );
    assert_eq!(
        runtime_label(false, false, true, false),
        "docker down — podman present"
    );
    assert_eq!(
        runtime_label(false, false, false, false),
        "no container runtime"
    );
}

#[test]
fn runtime_label_notes_installed_but_stopped_docker_cli() {
    assert_eq!(
        runtime_label(false, false, false, true),
        "docker installed but not running"
    );
}

#[test]
fn quiet_failure_leads_with_the_reason() {
    let text = quiet_failure(
        "docker",
        &["pull", "--quiet", "x"],
        Some(1),
        "pulling\nError response from daemon: manifest unknown\n",
    );
    assert_eq!(
        text.lines().next(),
        Some("docker pull failed (exit 1): Error response from daemon: manifest unknown")
    );
    assert_eq!(quiet_failure("docker", &["pull"], Some(1), ""), "docker pull failed (exit 1)");
}

#[test]
fn domain_images_always_pull_the_floating_tag() {
    assert!(super::images::composition_tag("web").ends_with(":latest-web"));
    assert!(super::images::is_missing_image("docker pull failed (exit 1): manifest unknown"));
    assert!(!super::images::is_missing_image("docker pull failed (exit 1): connection refused"));
}

#[test]
fn stale_daemon_files_are_replaced_once_then_left_alone() {
    let root = std::env::temp_dir().join(format!("koma-payload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dest = root.as_path();
    let catalog = dest.join("koma_sec_daemon").join("catalog.py");

    // A fresh home and an older version's files both get this binary's payload.
    std::fs::create_dir_all(catalog.parent().unwrap()).unwrap();
    std::fs::write(&catalog, "OLD").unwrap();
    std::fs::write(dest.join(".payload"), "0.0.1-0000000000000000").unwrap();
    assert!(ensure_daemon_sources_at(dest).unwrap());
    assert_ne!(std::fs::read_to_string(&catalog).unwrap(), "OLD");
    assert!(dest.join("koma_sec_daemon").join("guest_probe.py").is_file());

    // Same payload: nothing is rewritten.
    std::fs::write(&catalog, "UNTOUCHED").unwrap();
    assert!(!ensure_daemon_sources_at(dest).unwrap());
    assert_eq!(std::fs::read_to_string(&catalog).unwrap(), "UNTOUCHED");
    let _ = std::fs::remove_dir_all(&root);
}

fn entry(digest: &str) -> ImageEntry {
    ImageEntry { tag: String::new(), digest: digest.into(), size_bytes: 1, pulled_at: String::new() }
}

#[test]
fn an_old_inventory_with_one_active_image_still_reads() {
    let inv: ImagesInventory = serde_json::from_str(
        r#"{"version":"0.4.14","active":"recon","images":{"recon":{"tag":"t","digest":"d","size_bytes":1,"pulled_at":""}}}"#,
    )
    .unwrap();
    assert_eq!(inv.active_ids(), vec!["recon".to_string()]);
    assert!(inv.is_active("recon"));
}

#[test]
fn the_active_set_keeps_order_and_skips_images_that_are_gone() {
    let mut inv = ImagesInventory::default();
    inv.images.insert("recon".into(), entry("d1"));
    inv.images.insert("web".into(), entry("d2"));
    inv.images.insert("pxn".into(), entry(""));
    inv.active_set = vec!["web".into(), "pxn".into(), "recon".into(), "web".into()];
    inv.active = "web".into();
    assert_eq!(inv.active_ids(), vec!["web".to_string(), "recon".to_string()]);
    assert!(!inv.is_active("pxn"));
    // An older koma only reads `active`: it still finds the primary.
    let json = serde_json::to_value(&inv).unwrap();
    assert_eq!(json["active"], "web");
}
