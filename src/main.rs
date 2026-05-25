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

    ipc::start(state).await?;

    Ok(())
}
