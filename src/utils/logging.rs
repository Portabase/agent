use crate::settings::CONFIG;
use once_cell::sync::Lazy;
use time::macros::format_description;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::time::LocalTime;
use tracing_subscriber::{EnvFilter, Registry, fmt, prelude::*};

const LOGS_DIRECTORY: &str = "/var/log/app";

static FILE_APPENDER: Lazy<(NonBlocking, WorkerGuard)> = Lazy::new(|| {
    let file_appender = RollingFileAppender::new(Rotation::DAILY, LOGS_DIRECTORY, "app.log");
    tracing_appender::non_blocking(file_appender)
});

pub fn init_logger() {
    std::fs::create_dir_all(LOGS_DIRECTORY).ok();

    let (writer, _guard) = &*FILE_APPENDER;

    let timer = LocalTime::new(format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second]"
    ));

    let file_layer = fmt::layer()
        .with_writer(writer.clone())
        .with_timer(timer.clone())
        .with_ansi(false)
        .with_target(false);

    let env_filter = EnvFilter::new(format!(
        "{},aws_sdk_s3=warn,aws_smithy_http=warn,aws_smithy_runtime=warn,aws_smithy_client=warn,aws_smithy_types=warn,reqwest=warn,hyper=warn,tokio=warn,h2=warn",
        CONFIG.log
    ));

    let term_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_timer(timer.clone())
        .with_ansi(true)
        .with_target(false)
        .with_filter(env_filter);

    let subscriber = Registry::default().with(file_layer).with(term_layer);

    tracing::subscriber::set_global_default(subscriber).expect("Failed to set global subscriber");

    tracing::info!(
        "Logger initialized | TZ={} | POOLING={}s",
        CONFIG.timezone,
        CONFIG.pooling
    );
}
