#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// User pasted URLs into the input box.
    UrlsPasted(String),
    /// User clicked the Start button.
    StartClicked,
    /// User clicked Stop/Finish.
    StopFinishClicked,
    /// UI/render tick to coalesce rendering.
    Tick,
    /// Fallback for placeholder wiring.
    NoOp,
}
