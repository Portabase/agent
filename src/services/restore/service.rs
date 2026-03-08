use std::sync::Arc;
use crate::core::context::Context;

pub struct RestoreService {
    pub ctx: Arc<Context>,
}

impl RestoreService {
    pub fn new(ctx: Arc<Context>) -> Self {
        Self { ctx }
    }
}