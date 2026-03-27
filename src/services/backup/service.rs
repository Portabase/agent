use crate::core::context::Context as CoreContext;
use std::sync::Arc;

pub struct BackupService {
    pub ctx: Arc<CoreContext>,
}

impl BackupService {
    pub fn new(ctx: Arc<CoreContext>) -> Self {
        Self { ctx }
    }
}
