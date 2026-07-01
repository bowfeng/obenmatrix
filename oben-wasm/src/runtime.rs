use std::collections::HashMap;
use std::sync::Arc;

use wasmtime::component::Component;
use wasmtime::Module;

use crate::error::{WasmError, Result};

/// Configuration for the WASM runtime.
#[derive(Clone)]
pub struct WasmRuntimeConfig {
    /// Maximum memory in bytes for each component (default: 64MB).
    pub max_memory: u64,
    /// Timeout per API call in milliseconds.
    pub call_timeout_ms: u64,
    /// Enable/disable compilation cache.
    pub cache_enabled: bool,
}

impl Default for WasmRuntimeConfig {
    fn default() -> Self {
        WasmRuntimeConfig {
            max_memory: 64 * 1024 * 1024, // 64MB
            call_timeout_ms: 5000,         // 5s
            cache_enabled: true,
        }
    }
}

/// A prepared WASM component ready for instantiation.
pub struct PreparedComponent {
    pub name: String,
    pub component: Component,
    pub module: Module,
}

impl Clone for PreparedComponent {
    fn clone(&self) -> Self {
        let bytes = self.component.serialize().expect("serialize component to clone");
        PreparedComponent {
            name: self.name.clone(),
            component: Component::from_binary(
                &self.component.engine(),
                &bytes,
            )
            .expect("serialized component should deserialize"),
            module: self.module.clone(),
        }
    }
}

/// The WASM runtime engine that manages component compilation and caching.
#[allow(dead_code)]
pub struct WasmRuntime {
    engine: wasmtime::Engine,
    config: WasmRuntimeConfig,
    components: tokio::sync::RwLock<HashMap<String, Arc<PreparedComponent>>>,
}

impl WasmRuntime {
    /// Create a new WASM runtime with the given configuration.
    pub fn new(config: WasmRuntimeConfig) -> Result<Self> {
        let config = if cfg!(test) {
            WasmRuntimeConfig {
                cache_enabled: false,
                ..config
            }
        } else {
            config
        };
        let engine = wasmtime::Engine::default();
        Ok(Self {
            engine,
            config,
            components: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Get the wasmtime Engine reference.
    pub fn engine(&self) -> &wasmtime::Engine {
        &self.engine
    }

    /// Prepare a WASM component from bytes (compile + cache).
    pub async fn prepare_component(
        &self,
        name: &str,
        wasm_bytes: &[u8],
    ) -> Result<Arc<PreparedComponent>> {
        let name = name.to_string(); // Owned copy for move into blocking task
        // Check cache first
        {
            let components = self.components.read().await;
            if let Some(comp) = components.get(&name) {
                tracing::debug!(name, "Using cached component");
                return Ok(Arc::clone(comp));
            }
        }

        // Compile in blocking thread
        let engine = self.engine.clone();
        let wasm_bytes = wasm_bytes.to_vec();

        let compiled = tokio::task::spawn_blocking(move || {
            let module = Module::new(&engine, &wasm_bytes)
                .map_err(|e| WasmError::Compilation(e.to_string()))?;
            let component = Component::new(&engine, &wasm_bytes)
                .map_err(|e| WasmError::Compilation(e.to_string()))?;
            Ok::<_, WasmError>(PreparedComponent {
                name,
                component,
                module,
            })
        })
        .await
        .map_err(|e| WasmError::Execute(format!("spawn_blocking panicked: {e}")))??;

        // Cache the prepared component
        self.components
            .write()
            .await
            .insert(compiled.name.clone(), Arc::new(compiled.clone()));

        tracing::info!(%compiled.name, "Prepared WASM component");
        Ok(Arc::new(compiled))
    }

    /// Get a prepared component by name (from cache).
    pub async fn get_component(&self, name: &str) -> Option<Arc<PreparedComponent>> {
        self.components.read().await.get(name).cloned()
    }

    /// List all prepared component names.
    pub async fn list_components(&self) -> Vec<String> {
        self.components.read().await.keys().cloned().collect()
    }
}
