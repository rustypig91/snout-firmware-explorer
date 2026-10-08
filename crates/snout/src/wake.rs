//! A repaint callback shared with update workers.
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct Wake(Option<Arc<dyn Fn() + Send + Sync>>);
impl Wake {
    pub fn new(f: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(f)))
    }
    pub fn signal(&self) {
        if let Some(f) = &self.0 {
            f();
        }
    }
}
