//! tracing-test, wherever the suite runs.
//!
//! Off the web this is tracing-test itself. In a page the subscriber it
//! installs is built without a clock: its default one reads `SystemTime`,
//! which a page does not have, and the first event a test logs panics.

#[cfg(not(target_arch = "wasm32"))]
pub use tracing_test::*;

#[cfg(target_arch = "wasm32")]
pub use tracing_test::traced_test;

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub mod internal {
    pub use tracing_test::internal::*;

    /// The subscriber tracing-test installs, less the timestamp.
    pub fn get_subscriber(
        mock_writer: MockWriter<'static>,
        env_filter: &str,
    ) -> tracing_core::Dispatch {
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(env_filter)
            .with_writer(mock_writer)
            .with_level(true)
            .with_ansi(false)
            .without_time()
            .into()
    }
}
