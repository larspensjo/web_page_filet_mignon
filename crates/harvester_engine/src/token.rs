pub trait TokenCounter: Send + Sync {
    fn count(&self, text: &str) -> u32;
}

/// Simple, deterministic whitespace tokenizer as a placeholder.
#[derive(Debug, Default, Clone, Copy)]
pub struct WhitespaceTokenCounter;

impl TokenCounter for WhitespaceTokenCounter {
    fn count(&self, text: &str) -> u32 {
        text.split_whitespace().count() as u32
    }
}
