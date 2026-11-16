use super::*;

#[test]
fn local_reconnect_preserves_extension_tools_and_registration_keeps_server_generation() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let manager = McpManager::connect_all(rt.handle(), &[]);
    let host = crate::app::ext::ExtHostManager::new(rt.handle());
    let tools = vec![koma_extension::protocol::ToolDef {
        name: "inspect".into(),
        description: "Inspect".into(),
        input_schema: serde_json::json!({"type":"object"}),
    }];
    manager.register_extension_tools("run.koma.scope", &tools, host);
    if let McpBackend::Local { snapshot, .. } = &manager.backend {
        assert_eq!(
            snapshot.lock().unwrap().generation,
            0,
            "registering tools must not cancel connecting MCP servers"
        );
    }
    manager.reconnect(&[]);
    assert_eq!(
        manager.advertise_cached().1,
        vec!["mcp__run_koma_scope__inspect"]
    );
    manager.purge_extension_tools("run.koma.scope");
    assert!(manager.tool_names().is_empty());
}

#[test]
fn proxy_preserves_and_dispatches_session_extension_tools_locally() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut manager = McpManager::connect_all(rt.handle(), &[]);
    let now = std::time::Instant::now();
    let inner = Arc::get_mut(&mut manager).unwrap();
    inner.backend = McpBackend::Proxy {
        handle: rt.handle().clone(),
        sock: "/unused-mcp-test-socket".into(),
        cache: Mutex::new((vec![], vec![])),
        proxy_errors: Mutex::new(HashMap::new()),
        call_timeout: Mutex::new(PROXY_IO_TIMEOUT),
    };
    *inner.advertise_cache_at.lock().unwrap() = Some(now);
    *inner.advertise_confirmed_empty_at.lock().unwrap() = Some(now);
    let ext = crate::app::ext::ExtHostManager::new(rt.handle());
    let tools = vec![koma_extension::protocol::ToolDef {
        name: "inspect".into(),
        description: "Inspect extension state".into(),
        input_schema: serde_json::json!({"type":"object"}),
    }];
    manager.register_extension_tools("run.koma.scope", &tools, ext.clone());
    manager.register_extension_tools("run.koma.scope", &tools, ext);
    let name = "mcp__run_koma_scope__inspect";
    assert_eq!(manager.tool_names(), vec![name]);
    assert_eq!(manager.tool_defs()[0].function.name, name);
    let (defs, names) = manager.advertise_cached();
    assert_eq!(defs.len(), 1);
    assert_eq!(names, vec![name]);
    // Empty/updated global-server snapshots cannot remove the local contribution.
    if let McpBackend::Proxy { cache, .. } = &manager.backend {
        *cache.lock().unwrap() = (vec![], vec![]);
    }
    assert_eq!(manager.advertise_cached().1, vec![name]);
    // Dispatch reaches this process's extension host, never the invalid MCP socket.
    let error = manager
        .execute_blocking(name, &serde_json::json!({}))
        .unwrap_err();
    assert!(error.contains("not running"), "{error}");
    assert!(!error.contains("global daemon"), "{error}");
    manager.purge_extension_tools("run.koma.scope");
    assert!(manager.tool_defs().is_empty());
    assert!(manager.advertise_cached().1.is_empty());
}

fn bogus(uuid: &str, name: &str) -> McpServerEntry {
    McpServerEntry {
        uuid: uuid.into(),
        name: name.into(),
        enabled: true,
        command: "/definitely/not/a/real/command".into(),
        ..Default::default()
    }
}

fn error_of(manager: &McpManager, uuid: &str) -> Option<String> {
    manager.server_errors().get(uuid).cloned()
}

async fn wait_for_error(manager: &McpManager, uuid: &str) -> String {
    for _ in 0..200 {
        if let Some(e) = error_of(manager, uuid) {
            return e;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("no error recorded for {uuid}");
}

#[tokio::test]
async fn restart_clears_the_error_and_bumps_only_that_servers_epoch() {
    let manager = McpManager::connect_all(&tokio::runtime::Handle::current(), &[]);
    let (a, b) = (bogus("a", "a"), bogus("b", "b"));
    manager.restart_server(&a);
    manager.restart_server(&b);
    wait_for_error(&manager, "a").await;
    wait_for_error(&manager, "b").await;
    let McpBackend::Local { snapshot, .. } = &manager.backend else {
        panic!("local manager expected");
    };
    {
        let mut snap = snapshot.lock().unwrap();
        snap.errors.insert("a".into(), "stale".into());
        assert_eq!(snap.server_epoch.get("a"), Some(&1));
    }
    manager.restart_server(&a);
    {
        let snap = snapshot.lock().unwrap();
        assert_eq!(snap.server_epoch.get("a"), Some(&2));
        assert_eq!(snap.server_epoch.get("b"), Some(&1));
        assert!(
            !snap.errors.contains_key("a"),
            "restart must drop the old error"
        );
        assert!(snap.errors.contains_key("b"), "other servers are untouched");
    }
    let fresh = wait_for_error(&manager, "a").await;
    assert_ne!(fresh, "stale");
}

#[test]
fn a_connect_started_before_a_restart_is_stale() {
    let mut snap = Snapshot::default();
    assert!(snap.connect_is_current("a", 0, 0));
    snap.server_epoch.insert("a".into(), 1);
    assert!(!snap.connect_is_current("a", 0, 0));
    assert!(snap.connect_is_current("a", 0, 1));
    assert!(snap.connect_is_current("b", 0, 0));
    snap.generation = 1;
    assert!(!snap.connect_is_current("a", 0, 1));
}

#[tokio::test]
async fn project_overlay_is_idempotent_hides_base_tools_and_tears_down() {
    let handle = tokio::runtime::Handle::current();
    let manager = McpManager::connect_all(&handle, &[]);
    let host = crate::app::ext::ExtHostManager::new(&handle);
    let tools = vec![koma_extension::protocol::ToolDef {
        name: "inspect".into(),
        description: String::new(),
        input_schema: serde_json::json!({"type":"object"}),
    }];
    manager.register_extension_tools("run.koma.scope", &tools, host);
    assert_eq!(manager.tool_names(), vec!["mcp__run_koma_scope__inspect"]);

    let servers = vec![bogus("p1", "proj")];
    manager.set_project_servers(servers.clone(), vec!["mcp__run_koma_scope__".into()]);
    assert!(
        manager.tool_names().is_empty(),
        "overridden base tools are hidden"
    );
    assert!(manager.tool_defs().is_empty());
    let first = manager.overlay().unwrap();
    manager.set_project_servers(servers.clone(), vec!["mcp__run_koma_scope__".into()]);
    assert!(
        Arc::ptr_eq(&first, &manager.overlay().unwrap()),
        "same input is a no-op"
    );
    wait_for_error(&manager, "p1").await;

    manager.set_project_servers(Vec::new(), Vec::new());
    assert!(manager.overlay().is_none());
    assert!(manager.server_errors().is_empty());
    assert_eq!(manager.tool_names(), vec!["mcp__run_koma_scope__inspect"]);
}
