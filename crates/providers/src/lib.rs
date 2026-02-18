pub mod anthropic;
pub mod auth;
pub mod google;
pub mod openai_compat;
pub mod registry;
pub mod router;
pub mod traits;
pub(crate) mod sse;
pub(crate) mod util;

// Re-exports for convenience.
pub use registry::ProviderRegistry;
pub use router::LlmRouter;
pub use traits::{ChatRequest, ChatResponse, EmbeddingsRequest, EmbeddingsResponse, LlmProvider};
