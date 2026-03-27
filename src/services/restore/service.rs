use crate::core::context::Context;
use std::sync::Arc;

pub struct RestoreService {
    pub ctx: Arc<Context>,
}

impl RestoreService {
    pub fn new(ctx: Arc<Context>) -> Self {
        Self { ctx }
    }
}
