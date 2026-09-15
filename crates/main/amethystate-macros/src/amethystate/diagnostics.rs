use proc_macro2::Span;
use syn::Error;

/// Everything wrong with one declaration, gathered before any of it is
/// reported.
///
/// A refusal that returns where it is found reports one mistake per compile.
/// Three typos in a struct are three rounds of fixing and rebuilding, and the
/// second and third are invisible until the first is gone.
#[derive(Default)]
pub(crate) struct Diagnostics {
    found: Option<Error>,
}

impl Diagnostics {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn at(&mut self, span: Span, message: impl std::fmt::Display) {
        self.push(Error::new(span, message));
    }

    pub(crate) fn push(&mut self, error: Error) {
        match &mut self.found {
            Some(held) => held.combine(error),
            None => self.found = Some(error),
        }
    }

    /// Everything found, as one error that reports each of them.
    pub(crate) fn finish(self) -> Result<(), Error> {
        match self.found {
            Some(errors) => Err(errors),
            None => Ok(()),
        }
    }
}
