# Initiative: Integrate Inference.net Model Provider

## Executive Summary
Integrate `inference.net` as a first-class, named model provider in `jcode`. The provider leverages the OpenAI-compatible API at `https://api.inference.net/v1`, employing standard Bearer token authentication. The goal is to provide a seamless integration where users can authenticate via `/login`, select models from a dynamically updated catalog in the model picker, and execute turns with full tool-use support.

## Goals & Success Criteria
- [ ] **Authentication**: User can successfully configure an API key via `/login`.
- [ ] **Model Discovery**: The model picker is populated via dynamic catalog fetching from `inference.net`.
- [ ] **Turn Completion**: Chat turns (text and tool-use) are correctly routed to the `inference.net` API.
- [ ] **Default Models**: High-performance models like `kimi-k3-fast` are pre-configured or easily selectable.
- [ ] **Reliability**: Proper handling of 401 (auth), 429 (rate limit), and 400 (invalid model/request) errors.

## Non-Goals
- Implementation of a custom WebSocket transport (use HTTPS unless specific performance requirements surface).
- Custom prompt caching logic beyond what is provided by the OpenAI-compatible interface.

## Technical Specification

### 1. API details
- **Base URL**: `https://api.inference.net/v1`
- **Protocol**: OpenAI-compatible `/v1/chat/completions`
- **Auth**: `Authorization: Bearer <API_KEY>`
- **Model Catalog**: Standard OpenAI `/v1/models` endpoint.

### 2. Architecture & Data Flow

#### Auth State Transition
`UI (/login)` $\rightarrow$ `Credential Store` $\rightarrow$ `InferenceProvider` $\rightarrow$ `HTTP Request (Authorization Header)`.
- Credentials should be stored in the project's standard auth store to allow for multi-account/environment switching.

#### Model Catalog Logic
`Request (prefetch_models)` $\rightarrow$ `GET /v1/models` $\rightarrow$ `Filter relevant models` $\rightarrow$ `Update Provider's active_model_list` $\rightarrow$ `Render in Model Picker`.
- Catalog fetches must be asynchronous and non-blocking.
- The result is cached and only refreshed on explicit request or auth change.

#### Transport Mapping
Use `jcode-provider-openai` internal helpers to map:
- `ChatMessage` $\rightarrow$ OpenAI JSON message objects.
- `ToolDefinition` $\rightarrow$ OpenAI function schemas.
- `StreamEvent` $\rightarrow$ `jcode` provider stream events.

### 3. Interface Changes
- **`/login`**: Add `Inference.net` as a selectable provider option.
- **Model Picker**: 
    - Add `Inference.net` as a provider group.
    - Provide a "Refresh Catalog" action that triggers `refresh_model_catalog`.
    - Display model display-names fetched from the API.

## Failure Modes & Recovery
| Failure | Root Cause | Recovery Behavior | jcode Error Mapping |
| :--- | :--- | :--- | :--- |
| **401 Unauthorized** | Invalid/Expired API Key | Trigger `/login` prompt for the Inference.net provider. | Map to `error_looks_like_credential_failure` in `fallback_pick.rs`. |
| **429 Too Many Requests** | Rate Limit Exceeded | Apply exponential backoff; notify user of rate limit status. | Map to `retry_after` logic in `jcode-provider-core/src/retry_after.rs`. |
| **400 Bad Request** | Invalid model ID or prompt too long | Log error; if model-related, trigger catalog refresh; if prompt-related, trigger compaction. | Map to `is_openai_encrypted_content_too_large_error` if size-related; otherwise fatal. |
| **5xx Server Error** | Inference.net Backend Issue | Standard provider retry logic; if persistent, notify user of service outage. | Map to `is_transient_transport_error` in `transport.rs`. |
| **Empty Catalog** | API failure or account restriction | Fall back to a hardcoded list of "powerhouse" models and warn user. | Trigger `SATEY_MODELS` list (e.g., `kimi-k3-fast`). |

### Safety Model List (Fallback)
If `prefetch_models` fails or returns an empty list, the provider will default to:
- `kimi-k3-fast` (Primary)
- `kimi-k3` (Fallback)

## Implementation Roadmap

### Phase 1: Foundation
- [ ] Create `jcode-provider-inference` crate or implement within a shared runtime crate.
- [ ] Define the `InferenceProvider` struct implementing the `Provider` trait.

### Phase 2: Provider Logic
- [ ] Implement `complete` using the OpenAI-compatible transport.
- [ ] Implement `available_models` and `prefetch_models` using the `/v1/models` endpoint.
- [ ] Implement auth logic (Bearer token injection).

### Phase 3: System Integration
- [ ] Register `InferenceProvider` in the global registry.
- [ ] Update `/login` UI and logic to support Inference.net key entry.
- [ ] Wire the provider into the model routing system.

### Phase 4: Verification
- [ ] **Unit Tests**: Mock API responses to verify request formatting and error handling.
- [ ] **Integration Tests**: Verify the flow from `/login` $\to$ Model Picker $\to$ First Turn.
- [ ] **Manual QA**: End-to-end check with a live API key.
