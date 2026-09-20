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
        sock: "/unused-mcp-test-socket".into(),
        cache: Mutex::new((vec![], vec![])),
        proxy_errors: Mutex::new(HashMap::new()),
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
