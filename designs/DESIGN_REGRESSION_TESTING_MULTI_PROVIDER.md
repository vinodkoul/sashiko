# DESIGN: Regression Testing for Multi-Provider AI and Git Isolation

## Context
Following the refactor to a trait-based AI architecture, the implementation of Git repository isolation, and the resolution of early concurrency and API-specific bugs, we need automated guards to ensure these critical paths do not regress.

## Testing Strategy

### 1. Git Environment Isolation (Unit Tests)
**Goal:** Verify that `git_command()` and `sync_git_command()` effectively protect against environment pollution.
- **Implementation:** Tests will set "poison" variables (e.g., `GIT_DIR`, `GIT_AUTHOR_NAME`) in the test process and verify they are absent in the resulting `Command` object.
- **Location:** `src/git_ops.rs`.

### 2. AI Translation Logic (Unit Tests)
**Goal:** Ensure the complex mapping between Sashiko's generic types and provider-specific APIs (currently Gemini) is lossless.
- **Implementation:** 
    - Round-trip tests for System instructions, Tool calls, and nested message roles.
    - **Critical Path:** Verify that `thought_signature` in tool calls is correctly captured from responses and re-played in subsequent requests.
- **Location:** `src/ai/gemini.rs`.

### 3. IPC Protocol Stability & Concurrency (Contract & Integration Tests)
**Goal:** Ensure the Parent (Reviewer) and Child (Worker) always agree on the JSON schema and that the pipe never deadlocks.
- **Implementation:** 
    - **Contract:** Tests that manually serialize/deserialize `AiRequest` and `AiResponse` using the exact `type` tags used in the pipe.
    - **Concurrency:** Integration test that simulates a slow AI response while the child process continues to spam log messages. Verify the Parent continues to drain the child's STDOUT without blocking.
- **Location:** `src/ai/mod.rs` and `src/reviewer.rs`.

### 4. Reactive Cache Refresh (Integration/Mock Tests)
**Goal:** Verify that Sashiko "self-heals" when an AI context cache expires (403 error).
- **Implementation:** 
    - Mock the `AiProvider` to return a `403 Permission Denied (CachedContent not found)` on the first call.
    - Assert that `active_cache_name` is cleared, `ensure_cache` is called, and the request is automatically retried.
    - **Concurrency:** Verify that if multiple requests fail with 403 simultaneously, they only trigger a *single* cache refresh call (debouncing logic).
- **Location:** `src/reviewer.rs` or a new integration test file.

### 5. Provider Factory & Secret Sourcing (Unit Tests)
**Goal:** Verify that API keys are correctly picked up from the environment based on the selected provider.
- **Implementation:** Mock environment variables and verify `create_provider` returns an appropriately configured client.
- **Location:** `src/ai/mod.rs`.

## Execution Phases
1. **Phase A:** Git Isolation Validation.
2. **Phase B:** AI Translation & Thought Signature Accuracy.
3. **Phase C:** IPC Concurrency & Flow Control.
4. **Phase D:** Reactive Cache Refresh & Debouncing.
5. **Phase E:** Factory Logic & Secret Sourcing.
