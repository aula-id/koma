//! Vendor-neutral memory provider messages for extension authors.
//!
//! These types describe the proposed contract. The host does not yet invoke it.

use serde::{Deserialize, Serialize};

/// Version of the memory provider contract defined in this module.
pub const MEMORY_PROTOCOL_VERSION: u32 = 1;

/// The contract and features an extension declares in its manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryProviderDef {
    pub version: u32,
    pub display_name: String,
    #[serde(default)]
    pub can_store: bool,
}

/// Identifiers assigned by the host. Each request carries its own scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryScope {
    pub project_id: String,
    pub session_id: String,
}

/// Request for context relevant to a user prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrieveRequest {
    pub scope: MemoryScope,
    pub query: String,
    pub max_bytes: u32,
}

/// One provider result. The host must enforce the response size limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrieveResponse {
    pub entries: Vec<MemoryEntry>,
}

/// A completed turn. Repeated delivery must use the same idempotency key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreRequest {
    pub scope: MemoryScope,
    pub idempotency_key: String,
    pub user_content: String,
    pub assistant_content: String,
}

/// Acknowledges durable storage of a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreResponse {
    pub idempotency_key: String,
}

/// Methods invoked by the host on a selected provider extension.
pub const RETRIEVE_METHOD: &str = "memory.retrieve";
pub const STORE_METHOD: &str = "memory.store";

/// Implement this trait to receive typed memory requests in an extension.
pub trait MemoryProvider {
    fn retrieve(&mut self, request: RetrieveRequest) -> Result<RetrieveResponse, String>;
    fn store(&mut self, request: StoreRequest) -> Result<StoreResponse, String>;
}

/// Dispatch a memory invocation. Returns `None` for unrelated extension calls.
pub fn dispatch<P: MemoryProvider>(
    provider: &mut P,
    method: &str,
    params: serde_json::Value,
) -> Option<serde_json::Value> {
    let result = match method {
        RETRIEVE_METHOD => serde_json::from_value(params)
            .map_err(|error| error.to_string())
            .and_then(|request| provider.retrieve(request))
            .and_then(|response| serde_json::to_value(response).map_err(|error| error.to_string())),
        STORE_METHOD => serde_json::from_value(params)
            .map_err(|error| error.to_string())
            .and_then(|request| provider.store(request))
            .and_then(|response| serde_json::to_value(response).map_err(|error| error.to_string())),
        _ => return None,
    };
    Some(result.unwrap_or_else(|error| serde_json::json!({ "error": error })))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Example;
    impl MemoryProvider for Example {
        fn retrieve(&mut self, request: RetrieveRequest) -> Result<RetrieveResponse, String> {
            Ok(RetrieveResponse {
                entries: vec![MemoryEntry {
                    id: request.scope.session_id,
                    content: request.query,
                }],
            })
        }
        fn store(&mut self, request: StoreRequest) -> Result<StoreResponse, String> {
            Ok(StoreResponse {
                idempotency_key: request.idempotency_key,
            })
        }
    }

    #[test]
    fn dispatch_validates_and_routes_requests() {
        let scope = MemoryScope {
            project_id: "p".into(),
            session_id: "s".into(),
        };
        let params = serde_json::to_value(RetrieveRequest {
            scope,
            query: "hello".into(),
            max_bytes: 100,
        })
        .unwrap();
        let mut provider = Example;
        let result = dispatch(&mut provider, RETRIEVE_METHOD, params).unwrap();
        assert_eq!(result["entries"][0]["id"], "s");
        assert_eq!(result["entries"][0]["content"], "hello");
        assert!(
            dispatch(&mut provider, STORE_METHOD, serde_json::json!({})).unwrap()["error"]
                .is_string()
        );
        assert!(dispatch(&mut provider, "other", serde_json::json!({})).is_none());
    }

    #[test]
    fn store_request_round_trips_scope_and_idempotency_key() {
        let request = StoreRequest {
            scope: MemoryScope {
                project_id: "project".into(),
                session_id: "session".into(),
            },
            idempotency_key: "turn-1".into(),
            user_content: "question".into(),
            assistant_content: "answer".into(),
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["scope"]["session_id"], "session");
        assert_eq!(value["idempotency_key"], "turn-1");
        assert_eq!(
            serde_json::from_value::<StoreRequest>(value).unwrap(),
            request
        );
    }
}
