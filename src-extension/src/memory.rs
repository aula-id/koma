//! Vendor-neutral memory provider messages for extension authors.
//!
//! These types describe the proposed contract. The host does not yet invoke it.

use serde::{Deserialize, Serialize};

/// Version of the memory provider contract defined in this module.
pub const MEMORY_PROTOCOL_VERSION: u32 = 1;

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

#[cfg(test)]
mod tests {
    use super::*;

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
