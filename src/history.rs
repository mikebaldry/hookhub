use crate::{
    forward_request, history_db::ItemId, http_client, prepare_local_url, HistoryCommands,
    HISTORY_DB,
};
use anyhow::Result;
use log::{error, info};
use url::Url;

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

async fn handle_delete(id: ItemId) -> Result<()> {
    HISTORY_DB.delete(&id).await?;

    info!("Item deleted");

    Ok(())
}

async fn handle_replay(id: ItemId, mut local: Url) -> Result<()> {
    let item = HISTORY_DB.get(&id).await?;

    match item {
        Some(item) => {
            prepare_local_url(&mut local)?;
            let http = http_client()?;

            let _ = forward_request(item.request, local.clone(), http.clone()).await;
        }
        None => {
            error!("{} not found", id);
        }
    };

    Ok(())
}

async fn handle_clear() -> Result<()> {
    HISTORY_DB.clear().await?;

    info!("History has been cleared");

    Ok(())
}
