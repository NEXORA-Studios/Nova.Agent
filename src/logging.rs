use anyhow::Result;
use tracing_subscriber::EnvFilter;

/// 初始化日志系统
///
/// - 文件输出到 logs/ 目录（按天滚动）
pub fn init() -> Result<()> {
    let log_dir = std::path::Path::new("logs");
    std::fs::create_dir_all(log_dir)?;

    let file_appender = tracing_appender::rolling::daily(log_dir, "nova-agent.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // 让 guard 不被 drop（否则日志写入会停止）
    std::mem::forget(_guard);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .init();

    Ok(())
}
