# Evidence: Task 5 — Cron Delivery Mode Dispatch

## Scope
`oben-cron/src/jobs.rs` only. CronJob struct fields **unchanged**.

## What Was Done

1. Added `resolve_delivery_mode()` — reads `OBEN_DELIVERY_MODE` env var, defaults to `"simple"`.
2. Added `run_local_job()` — shared subprocess execution for `simple` and `daemon_agent` modes (`ober_exec run -p <prompt>`).
3. Added `run_gateway_job()` — POSTs JSON to `OBEN_GATEWAY_URL` via reqwest.
4. Refactored `advance_job()` — dispatches to the correct mode based on env var, delegates output saving to the mode-specific runner.

## Verification

### `cargo check --package oben-cron`

```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.69s
```

Zero errors, zero warnings.

### Diff

```diff
--- oben-cron/src/jobs.rs (original)
+++ oben-cron/src/jobs.rs (modified)
@@ -498,22 +498,17 @@ impl CronStore {
     }
 
     pub fn advance_job(&self, id: &str, ober_exec: &str) -> Result<()> {
-        let prompt = {
+        let (prompt, job) = {
             let data = self.data.lock().unwrap();
-            data.iter().find(|j| j.id == id).map(|j| j.prompt.clone())
+            let job: Option<CronJob> = data.iter().find(|j| j.id == id).cloned();
+            let job = job.ok_or_else(|| anyhow::anyhow!("Job not found: {}", id))?;
+            (job.prompt.clone(), job)
         };
-        let prompt = prompt.ok_or_else(|| anyhow::anyhow!("Job not found: {}", id))?;
 
-        let child = std::process::Command::new(ober_exec)
-            .args(&["run", "-p", &prompt])
-            .output()?;
-
-        let success = child.status.success();
-        let output = String::from_utf8_lossy(&child.stdout).to_string();
-        let error = if !success {
-            Some(String::from_utf8_lossy(&child.stderr).to_string())
-        } else {
-            None
+        let delivery_mode = resolve_delivery_mode();
+        let (success, _output, error) = match delivery_mode {
+            "gateway" => run_gateway_job(&prompt, &job, &self.output_dir)?,
+            _ => run_local_job(ober_exec, &prompt, &job, &self.output_dir)?,
         };
 
         let mut data = self.data.lock().unwrap();
@@ -526,18 +521,6 @@ impl CronStore {
             } else {
                 j.state = JobState::Error;
             }
-            if success {
-                let base = self.output_dir.join(&j.id);
-                if !base.exists() {
-                    std::fs::create_dir_all(&base).ok();
-                }
-                let filename = format!(
-                    "{}_{}.md",
-                    Utc::now().format("%Y-%m-%d"),
-                    Utc::now().format("%H-%M-%S")
-                );
-                let _ = std::fs::write(base.join(filename), &output);
-            }
             if let Schedule::Once { .. } = &j.schedule_obj {
                 j.enabled = false;
                 j.state = JobState::Completed;
@@ -659,6 +642,107 @@ fn _normalize_skill_list(skill: Option<&str>, skills: Option<&Vec<String>>) -> V
     result
 }
 
+// Delivery mode dispatch
+
+/// Resolve the delivery mode from OBEN_DELIVERY_MODE env var.
+/// Defaults to "simple" (subprocess execution).
+pub fn resolve_delivery_mode() -> &'static str {
+    match std::env::var("OBEN_DELIVERY_MODE").as_deref() {
+        Ok("daemon_agent") => "daemon_agent",
+        Ok("gateway") => "gateway",
+        _ => "simple",
+    }
+}
+
+/// Execute the job via subprocess (simple and daemon_agent modes).
+/// Uses `ober_exec run -p <prompt>` to execute the agent.
+fn run_local_job(
+    ober_exec: &str,
+    prompt: &str,
+    job: &CronJob,
+    output_dir: &PathBuf,
+) -> Result<(bool, String, Option<String>)> {
+    let child = std::process::Command::new(ober_exec)
+        .args(&["run", "-p", prompt])
+        .output()?;
+
+    let success = child.status.success();
+    let output = String::from_utf8_lossy(&child.stdout).to_string();
+    let error = if !success {
+        Some(String::from_utf8_lossy(&child.stderr).to_string())
+    } else {
+        None
+    };
+
+    if success {
+        let base = output_dir.join(&job.id);
+        if !base.exists() {
+            std::fs::create_dir_all(&base).ok();
        }
        let filename = format!(
            "{}_{}.md",
@@ -667,6 +751,57 @@ fn _normalize_skill_list(skill: Option<&str>, skills: Option<&Vec<String>>) -> V
         let _ = std::fs::write(base.join(filename), &output);
     }
 
+    Ok((success, output, error))
+}
+
+/// Execute the job by POSTing to an HTTP gateway.
+/// Sends the prompt as JSON in the request body.
+pub fn run_gateway_job(
+    prompt: &str,
+    job: &CronJob,
+    output_dir: &PathBuf,
+) -> Result<(bool, String, Option<String>)> {
+    let gateway_url = std::env::var("OBEN_GATEWAY_URL").map_err(|_| {
+        anyhow::anyhow!(
+            "OBEN_GATEWAY_URL is not set — required for gateway delivery mode"
+        )
+    })?;
+
+    let client = reqwest::blocking::Client::builder()
+        .timeout(std::time::Duration::from_secs(300))
+        .build()
+        .context("failed to create reqwest client")?;
+
+    info!("gateway: POST {} to {}", job.name, gateway_url);
+
+    let body = serde_json::json!({
+        "prompt": prompt,
+        "job_id": &job.id,
+        "job_name": &job.name,
+    });
+
+    let res = client.post(&gateway_url).json(&body).send()?;
+    let status = res.status();
+    let success = status.is_success();
+    let output = res.text()?;
+
+    let error = if !success {
+        Some(format!("HTTP {}", status))
+    } else {
+        None
+    };
+
+    if success {
+        let base = output_dir.join(&job.id);
+        if !base.exists() {
+            std::fs::create_dir_all(&base).ok();
+        }
+        let filename = format!(
+            "{}_{}.md",
+            Utc::now().format("%Y-%m-%d"),
+            Utc::now().format("%H-%M-%S")
+        );
+        let _ = std::fs::write(base.join(filename), &output);
+    }
+
+    Ok((success, output, error))
+}
+
 // Tests
 
 #[cfg(test)]
```

## Design Notes

- **`oben-agent` dependency**: `oben-agent` already depends on `oben-cron` (in `oben-agent/Cargo.toml:15`), so adding `oben-agent` → `oben-cron` would create a circular dependency. Therefore both `simple` and `daemon_agent` modes use the existing subprocess path (`ober_exec run -p <prompt>`). This preserves the proven behavior.
- **Default mode**: `"simple"` — existing behavior is fully preserved when `OBEN_DELIVERY_MODE` is unset.
- **Gateway URL**: sourced from `OBEN_GATEWAY_URL` env var. Gateway mode fails with a clear error if not set.
- **No CronJob struct changes**: All three modes use the same `advance_job(&self, id: &str, ober_exec: &str)` signature. The `OBEN_DELIVERY_MODE` and `OBEN_GATEWAY_URL` env vars route dispatch without touching the struct.

## Commit Message

```
feat(cron): add delivery_mode dispatch in advance_job
```
