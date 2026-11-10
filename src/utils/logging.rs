use crate::settings::CONFIG;
use std::sync::LazyLock;
use time::macros::format_description;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::time::LocalTime;
use tracing_subscriber::{EnvFilter, Registry, fmt, prelude::*};

const LOGS_DIRECTORY: &str = "/var/log/app";
const LOG_FILES_KEPT: usize = 7;

static FILE_APPENDER: LazyLock<(NonBlocking, WorkerGuard)> = LazyLock::new(|| {
    std::fs::create_dir_all(LOGS_DIRECTORY).ok();
    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("app.log")
        .max_log_files(LOG_FILES_KEPT)
        .build(LOGS_DIRECTORY)
        .expect("Failed to initialize log file appender");
    tracing_appender::non_blocking(file_appender)
});

pub fn init_logger() {
    let timer = LocalTime::new(format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second]"
    ));

    let file_layer = CONFIG.log_to_file.then(|| {
        let (writer, _guard) = &*FILE_APPENDER;
        fmt::layer()
            .with_writer(writer.clone())
            .with_timer(timer.clone())
            .with_ansi(false)
            .with_target(false)
    });

    let env_filter = EnvFilter::new(format!(
        "{},aws_sdk_s3=warn,aws_smithy_http=warn,aws_smithy_runtime=warn,aws_smithy_client=warn,aws_smithy_types=warn,reqwest=warn,hyper=warn,tokio=warn,h2=warn",
        CONFIG.log
    ));

    let term_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_timer(timer)
        .with_ansi(true)
        .with_target(false);

    let subscriber = Registry::default()
        .with(env_filter)
        .with(file_layer)
        .with(term_layer);

    tracing::subscriber::set_global_default(subscriber).expect("Failed to set global subscriber");

    tracing::info!(
        "Logger initialized | TZ={} | POLLING={}s | LOG_TO_FILE={}",
        CONFIG.timezone,
        CONFIG.polling,
        CONFIG.log_to_file
    );
}
