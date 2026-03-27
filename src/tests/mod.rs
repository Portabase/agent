mod domain;
mod services;
mod utils;

use once_cell::sync::Lazy;
use tracing_subscriber;

static TRACING: Lazy<()> = Lazy::new(|| {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("debug")
        .try_init();
});

fn init_tracing_for_test() -> () {
    Lazy::force(&TRACING);
}
