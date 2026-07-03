# Proof of Concept: delivery_mode in AppConfig

## Diff of Changes

```diff
diff --git a/oben-config/src/config.rs b/oben-config/src/config.rs
index 4546b8c..9adc2ba 100644
--- a/oben-config/src/config.rs
+++ b/oben-config/src/config.rs
@@ -31,6 +31,7 @@ pub struct AppConfig {
     pub fallback_models: Vec<FallbackConfig>,
     pub agent: AgentConfig,
     pub events: EventsConfig,
+    pub delivery_mode: DeliveryMode,
 }
 
 /// Configuration for vision/image analysis.
@@ -758,6 +759,21 @@ impl Default for EventsConfig {
     }
 }
 
+#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
+#[serde(rename_all = "snake_case")]
+pub enum DeliveryMode {
+    Simple,
+    #[serde(rename = "daemon-agent")]
+    DaemonAgent,
+    Gateway,
+}
+
+impl Default for DeliveryMode {
+    fn default() -> Self {
+        Self::Simple
+    }
+}
+
 impl Default for AppConfig {
     fn default() -> Self {
         Self {
@@ -804,6 +820,7 @@ impl Default for AppConfig {
             fallback_models: Vec::new(),
             agent: AgentConfig::default(),
             events: EventsConfig::default(),
+            delivery_mode: DeliveryMode::default(),
         }
     }
 }
```

## Test Results

### `cargo check --package oben-config` (PASS)

```
Checking oben-models v0.1.0 (/Users/ellie/workspace/oben-alien/oben-models)
Checking oben-transport v0.1.0 (/Users/ellie/workspace/oben-alien/oben-transport)
Checking oben-config v0.1.0 (/Users/ellie/workspace/oben-alien/oben-config)
Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.53s
```

### Test Suite (12 passed, 0 failed)

```
running 12 tests
test config::tests::test_default_settings                              ... ok
test config::tests::test_default_model_is_openrouter_qwen             ... ok
test config::tests::test_default_system_prompt_not_empty              ... ok
test config::tests::test_delivery_mode_daemon_agent                   ... ok
test config::tests::test_delivery_mode_missing_defaults_to_simple     ... ok
test config::tests::test_minimal_config_deserialize                   ... ok
test config::tests::test_providers_field_serializes_empty             ... ok
test config::tests::test_gateway_config_roundtrip_with_gateway        ... ok
test config::tests::test_gateway_config_qq_bot_serialization          ... ok
test config::tests::test_gateway_config_roundtrip_qq_bot              ... ok
test config::tests::test_config_yaml_roundtrip                        ... ok
test config::tests::test_save_load_roundtrip                          ... ok

test result: ok. 12 passed; 0 failed
```

### New Test: `test_delivery_mode_daemon_agent`

YAML input: `delivery_mode: daemon-agent`

Test deserializes a `Wrapper` struct containing `delivery_mode: DeliveryMode`, verifying that YAML value `"daemon-agent"` maps to Rust enum variant `DeliveryMode::DaemonAgent`.

```rust
#[test]
fn test_delivery_mode_daemon_agent() {
    let yaml = r#"delivery_mode: daemon-agent
"#;
    let w: Wrapper = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(w.delivery_mode, DeliveryMode::DaemonAgent);
}
```

Result: **PASS** - `DeliveryMode::DaemonAgent` successfully deserialized.

### New Test: `test_delivery_mode_missing_defaults_to_simple`

YAML input: (empty - no `delivery_mode` field)

Test uses a wrapper struct with `#[serde(default)]` on the `delivery_mode` field, verifying that when the field is absent from YAML, it defaults to `DeliveryMode::Simple` (since `DeliveryMode::default()` returns `Simple`).

```rust
#[test]
fn test_delivery_mode_missing_defaults_to_simple() {
    let yaml = r#"# no delivery_mode specified
"#;
    let w: Wrapper = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(w.delivery_mode, DeliveryMode::Simple);
}
```

Result: **PASS** - Missing field defaults to `DeliveryMode::Simple`.

### Serde Derivation Verification

`DeliveryMode` uses the required derive macros:
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Simple,
    #[serde(rename = "daemon-agent")]  // explicit override (snake_case would give "daemon_agent")
    DaemonAgent,
    Gateway,
}
```

## Summary

- New `DeliveryMode` enum: **Simple**, **DaemonAgent**, **Gateway** — all serializable to/from YAML strings
- Default value: `DeliveryMode::Simple` (via `impl Default`)
- `AppConfig` now contains `pub delivery_mode: DeliveryMode` field with default value
- Only modified `oben-config/src/config.rs` — no other crate touched
- No config migration code — field added with `#[serde(default)]` via struct-level `#[serde(default)]` on `AppConfig`
- All 12 tests pass including 2 new delivery_mode tests
