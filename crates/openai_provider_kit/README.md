# openai_provider_kit

Small Rust client abstraction for OpenAI chat-style API calls.

The crate currently focuses on Chat Completions-compatible requests. It owns the reusable OpenAI provider code, request and response types, model discovery, error mapping, and optional test support that used to live inside Harvester.

The consuming application is responsible for obtaining and storing API keys. `OpenAiProvider::new(api_key)` accepts an API key string directly. `OpenAiProvider::from_env()` is only a convenience helper for applications that already use `OPENAI_API_KEY`.

## Installation

During early development, depend on the Git repository directly:

```toml
[dependencies]
openai_provider_kit = { git = "https://github.com/larspensjo/rs-openai-provider-kit", tag = "v0.1.0" }
```

While working inside the Harvester workspace, the crate is consumed through a local path dependency.

## Simple Completion

```rust,no_run
use openai_provider_kit::{
    ChatMessage, ChatRole, LlmProvider, LlmRequest, ModelId, OpenAiProvider, ProviderKind,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENAI_API_KEY")?;
    let provider = OpenAiProvider::new(api_key);

    let request = LlmRequest::new(
        ModelId::new(ProviderKind::OpenAi, "gpt-4.1"),
        vec![
            ChatMessage::new(ChatRole::System, "Answer concisely."),
            ChatMessage::new(ChatRole::User, "What is the capital of Sweden?"),
        ],
    )
    .with_max_output_tokens(64);

    let response = provider.complete(&request).await?;
    println!("{}", response.content());

    Ok(())
}
```

## Model Listing

```rust,no_run
use openai_provider_kit::{LlmProvider, OpenAiProvider};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = OpenAiProvider::from_env()?;

    for model in provider.list_models().await? {
        println!("{model}");
    }

    Ok(())
}
```

Model listing filters the provider response down to rolling chat-capable model identifiers and excludes dated snapshots, embeddings, audio, image, realtime, search, and instruct models.

## Error Handling

Provider calls return `LlmError`, which keeps common operational categories distinct:

```rust,no_run
use openai_provider_kit::{
    ChatMessage, ChatRole, LlmError, LlmProvider, LlmRequest, ModelId, OpenAiProvider,
    ProviderKind,
};

#[tokio::main]
async fn main() {
    let provider = OpenAiProvider::new("application-supplied-key".to_string());
    let request = LlmRequest::new(
        ModelId::new(ProviderKind::OpenAi, "gpt-4.1"),
        vec![ChatMessage::new(ChatRole::User, "Hello")],
    );

    match provider.complete(&request).await {
        Ok(response) => println!("{}", response.content()),
        Err(LlmError::RateLimited { retry_after_secs }) => {
            eprintln!("rate limited; retry after {retry_after_secs:?} seconds");
        }
        Err(err) if err.is_retryable() => {
            eprintln!("transient provider failure: {err}");
        }
        Err(err) => {
            eprintln!("provider request failed: {err}");
        }
    }
}
```

## Features

- `rustls` is enabled by default for Reqwest TLS.
- `native-tls` enables Reqwest native TLS instead.
- `test-support` exposes reusable mock providers for downstream tests.
- `reqwest-passthrough` exposes `OpenAiProvider::with_client(reqwest::Client)` for consumers that intentionally want Reqwest client injection as part of their dependency surface.

## Testing

Normal CI does not require a live OpenAI API key. Live tests remain ignored and should be run explicitly after setting `OPENAI_API_KEY`.

```powershell
cargo test -p openai_provider_kit
cargo test -p openai_provider_kit -- --ignored
```

## Minimum Supported Rust Version

This crate follows the workspace minimum supported Rust version: Rust 1.83.

## License

Licensed under the MIT License. See [LICENSE](LICENSE).
