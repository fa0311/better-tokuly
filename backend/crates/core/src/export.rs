//! Personal data export.
//!
//! Each domain registers a [`DataExporter`] contributing its slice of a
//! user's data; the identity service assembles them into one archive when a
//! user requests an export.
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use uuid::Uuid;

use crate::error::AppResult;

#[async_trait]
pub trait DataExporter: Send + Sync + 'static {
    /// Section name in the archive, e.g. `"chat"`.
    fn name(&self) -> &'static str;
    /// Everything this domain stores about the user, as JSON.
    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value>;
}

#[derive(Default)]
pub struct Exporters {
    list: RwLock<Vec<Arc<dyn DataExporter>>>,
}

impl Exporters {
    pub fn register(&self, exporter: Arc<dyn DataExporter>) {
        self.list.write().push(exporter);
    }

    pub fn all(&self) -> Vec<Arc<dyn DataExporter>> {
        self.list.read().clone()
    }
}
