use openai_provider_kit::{
    ChatMessage, ChatRole, LlmError, LlmProvider, LlmRequest, ModelId, OpenAiProvider, ProviderKind,
};

#[tokio::main]
async fn main() -> Result<(), LlmError> {
    let provider = OpenAiProvider::from_env()?;

    let request = LlmRequest::new(
        ModelId::new(ProviderKind::OpenAi, "gpt-4.1"),
        vec![
            ChatMessage::new(ChatRole::System, "Answer concisely."),
            ChatMessage::new(ChatRole::User, "Write one sentence about Rust ownership."),
        ],
    )
    .with_max_output_tokens(80);

    let response = provider.complete(&request).await?;
    println!("{}", response.content());
    println!(
        "model={} input_tokens={} output_tokens={} cached_input_tokens={}",
        response.model_id().model_name(),
        response.usage().input_tokens,
        response.usage().output_tokens,
        response.usage().cached_input_tokens
    );

    Ok(())
}
