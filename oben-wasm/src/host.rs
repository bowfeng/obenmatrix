use wasmtime::component::Linker;
use wasmtime::Engine;

use crate::error::Result;

/// WASM host runtime environment for plugins.
pub struct HostRuntime {
    linker: Linker<()>,
}

impl HostRuntime {
    /// Create a new WASM host linker with all exported functions.
    pub fn new(engine: &Engine) -> Result<Self> {
        let linker = Linker::new(engine);
        Ok(HostRuntime { linker })
    }

    /// Get the linker reference for use with instantiations.
    pub fn linker(&self) -> &Linker<()> {
        &self.linker
    }
}
