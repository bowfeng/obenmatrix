# Evidence: HTTP Request/Response Structs (oben-cron)

## Task

Add HTTP request/response structs to `oben-cron/src/http.rs` — no server logic.

---

## 1. Cargo Check Pass

```
$ cargo check --package oben-cron
    Blocking waiting for file lock on build directory
    Checking regex-automata v0.4.14
   Compiling syn v2.0.117
    Checking uuid v1.23.2
    Checking matchers v0.2.0
    Checking regex v1.12.3
   Compiling serde_derive v1.0.228
   Compiling tracing-attributes v0.1.31
   Compiling tokio-macros v2.7.0
    Checking tokio v1.52.3
    Checking tracing v0.1.44
    Checking serde v1.0.228
    Checking tracing-subscriber v0.3.23
    Checking chrono v0.4.44
    Checking cron v0.15.0
    Checking oben-cron v0.1.0 (/Users/ellie/workspace/oben-alien/oben-cron)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 18.22s
```

Zero errors.

---

## 2. Unit Tests — Serde Roundtrip

```
$ cargo test --package oben-cron --lib http
running 2 tests
test http::tests::test_submit_request_roundtrip ... ok
test http::tests::test_missing_prompt_fails ... ok

test result: ok. 2 passed; 0 failed
```

### Test 1: Roundtrip Serialize → Deserialize = Same

`test_submit_request_roundtrip` serializes a `CronSubmitRequest` with all fields populated
(`prompt`, `deliver_target: Some(DeliverTarget::Origin)`, `session_id: Some("sess-abc")`),
then deserializes the JSON and asserts all fields match the original.

### Test 2: Missing Prompt Field Fails

`test_missing_prompt_fails` deserializes `{"deliver_target":null}` (no `prompt`) and asserts
the result is an error — confirming `prompt` is a required field.

---

## 3. Files Changed

### `oben-cron/src/http.rs` (NEW)

```rust
//! HTTP request/response data types for the cron scheduler.
//!
//! These structs are pure data types — no HTTP server logic lives here.

use serde::{Deserialize, Serialize};

/// Request body for submitting a cron job via HTTP.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CronSubmitRequest {
    /// The prompt to execute.
    pub prompt: String,
    /// Where to deliver the result.
    #[serde(default)]
    pub deliver_target: Option<crate::DeliverTarget>,
    /// Optional session ID to associate with the job.
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Response returned after a cron job is submitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSubmitResponse {
    /// Unique job identifier.
    pub job_id: String,
    /// Current job status.
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serde roundtrip: serialize then deserialize — should produce the same value.
    #[test]
    fn test_submit_request_roundtrip() {
        let original = CronSubmitRequest {
            prompt: "run analysis".into(),
            deliver_target: Some(crate::DeliverTarget::Origin),
            session_id: Some("sess-abc".into()),
        };

        let json = serde_json::to_string(&original).expect("serialize");
        let roundtrip: CronSubmitRequest =
            serde_json::from_str(&json).expect("deserialize");

        assert_eq!(roundtrip.prompt, original.prompt);
        assert_eq!(
            roundtrip.deliver_target,
            original.deliver_target,
            "deliver_target should survive roundtrip"
        );
        assert_eq!(
            roundtrip.session_id, original.session_id,
            "session_id should survive roundtrip"
        );
    }

    /// Missing `prompt` must fail deserialization.
    #[test]
    fn test_missing_prompt_fails() {
        let json = r#"{"deliver_target":null}"#;
        let result: Result<CronSubmitRequest, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "deserialization must fail when required `prompt` field is missing"
        );
    }
}
```

### `oben-cron/src/lib.rs` (MODIFIED — +1 line)

```diff
+pub mod http;
 pub mod jobs;
 pub mod schedule;
```

---

## 4. Struct Fields Verified

| Struct | Field | Type | Notes |
|--------|-------|------|-------|
| `CronSubmitRequest` | `prompt` | `String` | Required |
| `CronSubmitRequest` | `deliver_target` | `Option<DeliverTarget>` | Optional, `#[serde(default)]` |
| `CronSubmitRequest` | `session_id` | `Option<String>` | Optional, `#[serde(default)]` |
| `CronSubmitResponse` | `job_id` | `String` | Set by server at submit time |
| `CronSubmitResponse` | `status` | `String` | Set by server at submit time |

Uses existing `crate::DeliverTarget` from `jobs.rs:36-46` — no new types added.
No reqwest, no server logic.
