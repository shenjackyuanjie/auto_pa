use std::sync::OnceLock;
use tracing::Level;
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{
    fmt::{self, writer::MakeWriterExt},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

static LOG_FILE_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct LoggerConfig {
    pub level: Level,
    pub file_log: bool,
    pub filename_suffix: String,
    pub logs_dir: String,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            level: Level::INFO,
            file_log: false,
            filename_suffix: "".to_string(),
            logs_dir: "./logs".to_string(),
        }
    }
}

impl LoggerConfig {
    pub fn with_level(self, level: Level) -> Self {
        Self { level, ..self }
    }

    pub fn with_file_log(self, file_log: bool) -> Self {
        Self { file_log, ..self }
    }

    pub fn with_filename_suffix(self, filename_suffix: &str) -> Self {
        Self {
            filename_suffix: filename_suffix.to_string(),
            ..self
        }
    }

    pub fn with_logs_dir(self, logs_dir: &str) -> Self {
        Self {
            logs_dir: logs_dir.to_string(),
            ..self
        }
    }

    pub fn init(self) {
        init(self);
    }
}

pub fn init(cfg: LoggerConfig) {
    let stdio = std::io::stdout.with_max_level(cfg.level);

    let stdout_layer = fmt::layer()
        .with_writer(stdio)
        .with_line_number(true)
        .with_thread_ids(true);

    // 初始化
    let registry = tracing_subscriber::registry().with(stdout_layer);

    if cfg.file_log {
        // 创建按天轮换的文件 appender
        let file_appender = {
            RollingFileAppender::builder()
                .filename_suffix(format!("{}.log", cfg.filename_suffix).as_str()) // 要后缀!
                .rotation(Rotation::DAILY)
                .build(cfg.logs_dir.as_str())
                .expect("Failed to build log file writer.")
        };
        let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);

        let _ = LOG_FILE_GUARD.set(guard);
        let file = non_blocking_file.with_max_level(cfg.level);
        let file_layer = fmt::layer().with_writer(file).with_ansi(false);
        registry.with(file_layer).init();
    } else {
        registry.init();
    }
}
