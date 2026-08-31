use crate::path::StorePath;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) const MAX_INTERCEPT_DEPTH: usize = 10;

pub struct InterceptGuard {
    depth: Arc<AtomicUsize>,
}

impl InterceptGuard {
    pub(crate) fn enter(depth: &Arc<AtomicUsize>, path: StorePath) -> Option<Self> {
        let prev = depth.fetch_add(1, Ordering::Acquire);
        if prev >= MAX_INTERCEPT_DEPTH {
            depth.fetch_sub(1, Ordering::Release);
            tracing::warn!(
                target: "amethystate::intercept",
                path = %path,
                depth = prev + 1,
                "maximum intercept depth reached, skipping execution"
            );
            None
        } else {
            Some(Self {
                depth: depth.clone(),
            })
        }
    }
}

impl Drop for InterceptGuard {
    fn drop(&mut self) {
        self.depth.fetch_sub(1, Ordering::Release);
    }
}

pub struct InterceptDisposer {
    pub id: u64,
    pub path: StorePath,
    pub(crate) cleanup: Arc<dyn Fn(u64) + Send + Sync + 'static>,
}

impl InterceptDisposer {
    pub fn remove(self) {
        (self.cleanup)(self.id);
    }
}

impl std::fmt::Debug for InterceptDisposer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InterceptDisposer")
            .field("id", &self.id)
            .field("path", &self.path)
            .finish()
    }
}
