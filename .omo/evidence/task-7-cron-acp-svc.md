# Task 7: Cron ACP Service — HTTP Server

## Summary

Added an HTTP server to the `oben-cron` crate that exposes a `POST /cron/submit` endpoint for submitting cron jobs programmatically.

## Files Changed

### New: `oben-cron/src/server.rs`
- HTTP server using **axum 0.8** with CORS support
- `POST /cron/submit` endpoint accepts `CronSubmitRequest`
- Validates prompts via `scan_cron_prompt` (injection/exfiltration checks)
- Creates `CronJob`, persists via `CronStore`
- Graceful shutdown on SIGINT

### Modified: `oben-cron/src/main.rs`
- Wrapped `CronStore` in `Arc` for sharing
- HTTP server starts on port 8790 (configurable via `OBEN_CRON_PORT`)
- Cron tick loop moved to background via `Daemon::spawn()`
- Main thread waits for signal then stops daemon cleanly

### Modified: `oben-cron/Cargo.toml`
Added dependencies:
```toml
axum = { version = "0.8", features = ["json"] }
tower = { version = "0.5", features = ["util"] }
tower-http = { version = "0.6", features = ["cors"] }
```

### Modified: `oben-cron/src/lib.rs`
Added `pub mod server;`

## Verification

### `cargo check --package oben-cron` — PASSED

```
    Checking oben-cron v0.1.0 (/Users/ellie/workspace/oben-alien/oben-cron)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.95s
```

No warnings, no errors.

### Cargo.toml dependencies verified

```toml
axum = { version = "0.8", features = ["json"] }
tower = { version = "0.5", features = ["util"] }
tower-http = { version = "0.6", features = ["cors"] }
```

## Diff

### `oben-cron/Cargo.toml`
```diff
+axum = { version = "0.8", features = ["json"] }
+tower = { version = "0.5", features = ["util"] }
+tower-http = { version = "0.6", features = ["cors"] }
```

### `oben-cron/src/lib.rs`
```diff
+pub mod server;
```

### `oben-cron/src/main.rs` (key changes)
```diff
+let store: Arc<oben_cron::jobs::CronStore> = Arc::new(store);

+// Start HTTP server for job submission
+let port: u16 = std::env::var("OBEN_CRON_PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8790);
+let addr = format!("127.0.0.1:{}", port);
+let server_store = Arc::clone(&store);
+info!("HTTP server starting on {}", addr);
+tokio::spawn(async move {
+    let parse_addr: std::net::SocketAddr = addr.parse().unwrap();
+    oben_cron::server::run_server(server_store, parse_addr).await;
+});

-// Main tick loop (inline)
+// Spawn cron tick loop as background task
+let daemon_handle = oben_cron::jobs::Daemon::spawn(daemon_store, Duration::from_secs(60));
```

### `oben-cron/src/server.rs` (new file)
- 128 lines
- `run_server()` — binds TCP listener, applies CORS layer, routes to `/cron/submit`
- `shutdown_signal()` — listens for SIGINT
- `submit_handler()` — validates → creates → persists cron job
- Unit test: serde roundtrip of `CronSubmitRequest`
