/// Renders a report into an `anyhow` error, keeping the whole tree of contexts
/// and attachments rather than only the outermost one.
pub fn anyhowed<E: std::fmt::Debug>(error: E) -> anyhow::Error {
    anyhow::anyhow!("{error:?}")
}
