//! AWS Bedrock stub adapter.
//!
//! Native Bedrock SigV4 authentication requires the `aws-sigv4` and
//! `aws-credential-types` crates, which add significant dependency weight.
//! This stub registers the `aws_bedrock` provider kind so that the config
//! option is recognized, but all runtime methods return an actionable error
//! directing users to Bedrock's OpenAI-compatible gateway instead.
//!
//! Users who need Bedrock today can use:
//! ```toml
//! [[llm.providers]]
//! id = "bedrock"
//! kind = "openai_compat"
//! base_url = "https://bedrock-runtime.us-east-1.amazonaws.com/v1"
//! ```
//! with IAM auth configured externally (e.g. IAM Roles Anywhere, credential
//! helper, or `aws-vault`).

use crate::traits::{
    ChatRequest, ChatResponse, EmbeddingsRequest, EmbeddingsResponse, LlmProvider,
};
use sa_domain::capability::LlmCapabilities;
use sa_domain::config::ProviderConfig;
use sa_domain::error::{Error, Result};
use sa_domain::stream::{BoxStream, StreamEvent};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

const STUB_MSG: &str = "\
AWS Bedrock native SigV4 auth is not yet implemented (requires the \
aws-sdk-bedrockruntime crate). Use kind = \"openai_compat\" with Bedrock's \
OpenAI-compatible endpoint instead: \
base_url = \"https://bedrock-runtime.<region>.amazonaws.com/v1\" \
and configure IAM credentials externally.";

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Adapter struct
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Stub LLM provider for AWS Bedrock.
///
/// All runtime methods return an error with guidance on how to use
/// Bedrock via the OpenAI-compatible gateway. The provider is registered
/// successfully so that configuration validation passes and the config
/// option is discoverable.
pub struct BedrockProvider {
    id: String,
    capabilities: LlmCapabilities,
}

impl BedrockProvider {
    /// Create the stub provider from config.
    ///
    /// This always succeeds so the provider appears in the registry, but
    /// all operational methods will return an error with guidance.
    pub fn from_config(cfg: &ProviderConfig) -> Result<Self> {
        tracing::warn!(
            provider_id = %cfg.id,
            "AWS Bedrock provider registered as a stub — native SigV4 auth \
             not yet implemented. Use kind = \"openai_compat\" with Bedrock's \
             OpenAI-compatible endpoint for now."
        );

        Ok(Self {
            id: cfg.id.clone(),
            capabilities: LlmCapabilities::default(),
        })
    }

    fn stub_error(&self) -> Error {
        Error::Provider {
            provider: self.id.clone(),
            message: STUB_MSG.into(),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Trait implementation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[async_trait::async_trait]
impl LlmProvider for BedrockProvider {
    async fn chat(&self, _req: &ChatRequest) -> Result<ChatResponse> {
        Err(self.stub_error())
    }

    async fn chat_stream(
        &self,
        _req: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent>>> {
        Err(self.stub_error())
    }

    async fn embeddings(&self, _req: EmbeddingsRequest) -> Result<EmbeddingsResponse> {
        Err(self.stub_error())
    }

    fn capabilities(&self) -> &LlmCapabilities {
        &self.capabilities
    }

    fn provider_id(&self) -> &str {
        &self.id
    }
}
