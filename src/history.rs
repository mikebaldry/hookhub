use crate::{HistoryCommands, HISTORY_DB};
use anyhow::Result;
use log::info;

pub async fn handle(command: HistoryCommands) -> Result<()> {
    match command {
        HistoryCommands::List => handle_list().await,
        HistoryCommands::Delete { id } => handle_delete(id).await,
        HistoryCommands::Clear => handle_clear().await,
        HistoryCommands::Replay { id, local } => handle_replay(id, local).await,
    }
}

async fn handle_list() -> Result<()> {
    let items = HISTORY_DB.list().await?;

    if items.is_empty() {
        info!("History is empty");
        return Ok(());
    }

    for item in items.iter() {
        info!(
            "[{} {}] {} {}",
            item.id, item.received_at, item.request.method, item.request.fullpath
        );
    }

    Ok(())
}

async fn handle_delete(id: u32) -> Result<()> {
    Ok(())
}

async fn handle_replay(id: u32, local: String) -> Result<()> {
    Ok(())
}

async fn handle_clear() -> Result<()> {
    Ok(())
}
