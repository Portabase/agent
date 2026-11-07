mod domain;
mod services;
mod settings_tests;
mod storage;
mod utils;

use std::sync::LazyLock;
use tracing_subscriber;

static TRACING: LazyLock<()> = LazyLock::new(|| {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("debug")
        .try_init();
});

fn init_tracing_for_test() -> () {
    LazyLock::force(&TRACING);
}
