# Memory provider extensions

> **Status: proposal.** This document describes a possible design. The memory provider interface and the events below are not implemented.

## Goal

Let a user select an extension to store and retrieve memory. Keep the current Markdown memory as the default. Do not require a network service for the default configuration.

A memory provider is an extension that implements a common contract. The extension can use files, a local process, or a remote service. The extension is responsible for the provider-specific integration.

## Current behavior

Koma stores project memory in a directory for the current working directory. The directory contains an index and one Markdown file for each memory. The system prompt receives the index. The `remember`, `recall`, and `forget` tools manage the entries.

Koma already has an extension host. Extensions can contribute tools and receive events. The existing event channel is fire-and-forget. It cannot by itself confirm that a provider saved a message or return memory for a prompt. The extension transport also has a frame size limit. See [Extension architecture](ARCH_EXTENSION.md) for the current contract.

## Proposed design

```text
Session lifecycle
      |
      v
Memory coordinator ---> Provider selection and policy
      |                         |
      +--> Built-in Markdown    +--> Memory extension
                                         |
                                         +--> Provider storage or service
```

The coordinator selects the provider for a session. It supplies project and session identity. It calls the provider at defined points in the session. The provider returns data or an error through request and response messages.

The built-in Markdown provider remains available without an extension. Existing memory files must continue to work. The user must explicitly enable an external provider. The initial design selects one provider per session. Support for multiple active providers is a separate decision.

## Provider contract

The following operations are a proposed minimum. Their names and wire format are not final.

| Operation | When | Input | Result |
|---|---|---|---|
| `initialize` | A session selects the provider | Contract version, project identity, session identity, and policy | Supported capabilities and provider status |
| `retrieve` | Before a prompt, when enabled | Query, scope, and a maximum response size | Relevant memory entries and source identifiers |
| `store` | After a turn or on a user request | Scoped content, metadata, and an idempotency key | Acknowledgment or an error |
| `finish` | A session ends | Session identity and final checkpoint | Acknowledgment or an error |

Providers can declare optional capabilities, such as search, deletion, and per-turn storage. The host must not assume that every provider has the same capabilities. The user interface must show which capabilities are available.

The host should use request and response calls for operations that need an acknowledgment. A notification alone does not prove that the provider saved data. The contract must define timeouts, error codes, size limits, cancellation, and version negotiation before implementation.

## Session flow

1. The user selects and enables a provider.
2. Koma initializes the provider for the session and checks its capabilities.
3. If automatic retrieval is enabled, Koma asks for relevant memory before a prompt. Koma applies the configured context limit.
4. If automatic storage is enabled, Koma sends the permitted content after each completed turn. Koma records an acknowledgment for each item.
5. When the session ends, Koma requests a final checkpoint. Koma does not rely only on a clean shutdown to preserve pending data.

If retrieval fails or times out, the session continues without the external memory. Koma shows the failure in provider status. It must not silently treat a failed storage operation as successful.

## Scope and user control

The provider must receive an explicit scope with every request. The scope identifies the user or profile, project, and session as applicable. The contract must define how linked worktrees and remote projects map to a project identity. It must not infer that two projects should share memory only because their folder names match.

The settings must identify the provider, storage location or service endpoint, and the data that can be sent. Automatic storage must require explicit user enablement. Users need controls to disable it, inspect status, and manage or delete memories when the provider supports those operations.

Treat retrieved memory as data. It must not gain the authority of system instructions. Apply size limits and identify the source so the user can inspect it. Do not include credentials in logs or error messages.

## Reliability

If Koma offers automatic storage, it needs a durable local queue for items that the provider has not acknowledged. Each item needs a stable idempotency key. A retry must use the same key. The provider contract must state how duplicate requests are handled.

A crash can occur between a turn and its acknowledgment. On restart, Koma must recover pending items without sending them to a different provider or scope. Queue contents can contain sensitive data. Protect them as session data and define retention and deletion behavior.

The implementation must handle a provider that is unavailable, slow, disabled, or removed. A provider failure must not block normal chat indefinitely. The current extension process restart behavior and transport limits must be addressed as part of the implementation, not assumed to be complete.

## Delivery plan

1. Define the provider manifest, versioned request and response types, scope, grants, and error model.
2. Add a coordinator and adapt the built-in Markdown store without changing existing behavior.
3. Add an example extension and user controls for provider selection and status.
4. Add automatic retrieval with a context limit and a timeout.
5. Add optional storage with acknowledgments, a durable queue, and crash recovery.
6. Test local and remote sessions, project isolation, restarts, duplicate delivery, provider failures, and migration from existing Markdown memory.

## Decisions still required

- The precise provider interface and compatibility policy.
- The grant and consent model for transcript access and network transmission.
- Which message types are included in automatic storage, and how users exclude data.
- Whether one or more providers can be active at once.
- Where the durable queue lives and how users delete it.
- How provider settings and secrets are stored.
- How a memory provider exposes inspection, correction, export, and deletion.
