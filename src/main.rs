mod console;
mod instance;
mod ipc;
mod logging;
mod process;
mod state;

use state::SharedState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logging::init()?;

    let state: SharedState = std::sync::Arc::new(state::AppState::new());

    // 监听 Ctrl+C
    let shutdown_state = state.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.expect("failed to listen for ctrl+c");
        tracing::info!("received shutdown signal");
        shutdown_state.instances.shutdown_all().await;
        std::process::exit(0);
    });

    ipc::start(state).await?;

    Ok(())
}
