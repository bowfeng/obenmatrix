/// Transport provider trait — abstracts LLM API calls.
///
/// Maps to `agent/transports/base.py` + individual transport implementations.
///
/// NOTE: The actual trait is defined in `oben-models::providers::TransportProvider`
/// to avoid circular dependencies. This module re-exports it.

pub use oben_models::providers::TransportProvider;
pub use oben_models::providers::TransportResponse;
pub use oben_models::providers::TransportToolCall;
