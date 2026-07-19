mod batch;
mod openai;
mod provider;
mod types;

#[cfg(feature = "test-support")]
pub mod test_support;

pub use batch::{
    parse_batch_output_jsonl, BatchHandle, BatchInputLine, BatchLifecycle, BatchOutputLine,
    BatchRequestCounts, BatchTransport, FileId,
};
pub use openai::openai_chat_completion_body;
pub use openai::OpenAiProvider;
pub use provider::LlmProvider;
pub use types::{
    ChatMessage, ChatRole, FinishReason, LlmError, LlmRequest, LlmResponse, ModelId, ProviderKind,
    ResponseFormat, TokenUsage,
};
