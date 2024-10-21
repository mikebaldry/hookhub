use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use glob::glob;
use hookhub::RequestMessage;
use log::{error, info};
use serde::{Deserialize, Serialize};
use tokio::{fs, io};
use url::Url;

use crate::{
    client::{forward_request, http_client},
    HistoryCommands, ROOT_PATH,
};

pub async fn handle(command: HistoryCommands) -> Result<()> {
    match command {
        HistoryCommands::List => handle_list().await,
        HistoryCommands::Delete { id } => handle_delete(id).await,
        HistoryCommands::Clear => handle_clear().await,
        HistoryCommands::Replay { id } => handle_replay(id).await,
    }
}

async fn handle_list() -> Result<()> {
    let items = History::new()?.list().await?;

    for item in items.iter() {
        info!(
            "[{} {}] {} {}",
            item.id, item.received_at, item.request.method, item.request.fullpath
        );
    }

    Ok(())
}

async fn handle_delete(id: String) -> Result<()> {
    History::new()?.delete(&id).await?;

    info!("Item deleted");

    Ok(())
}

async fn handle_replay(id: String) -> Result<()> {
    let item = History::new()?.get(&id).await?;

    match item {
        Some(item) => {
            let http = http_client()?;

            let _ = forward_request(item.request, item.local.clone(), http.clone()).await;
        }
        None => {
            error!("{} not found", id);
        }
    };

    Ok(())
}

async fn handle_clear() -> Result<()> {
    History::new()?.clear().await?;

    info!("History has been cleared");

    Ok(())
}

#[derive(Serialize, Deserialize, Default)]
pub struct History {
    path: PathBuf,
}

impl History {
    pub fn new() -> Result<Self> {
        let path = ROOT_PATH.join("history");

        match std::fs::create_dir(&path) {
            Ok(_) => Ok(Self { path: path.clone() }),
            Err(e) => {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(Self { path: path.clone() })
                } else {
                    Err(e.into())
                }
            }
        }
    }

    pub async fn get(&self, id: &String) -> Result<Option<Item>> {
        let path = self.path.join(format!("{}.json", id));

        self.read(&path).await
    }

    pub async fn add(&self, item: &Item) -> Result<String> {
        let mut generator = names::Generator::default();
        let id = generator.next().unwrap();
        let path = self.path.join(format!("{}.json", id));

        let data = serde_json::to_vec(item)?;

        match fs::write(path, data).await {
            Ok(_) => Ok(id),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn list(&self) -> Result<Vec<Item>> {
        let results = glob(self.path.join("*.json").to_str().unwrap())?
            .map(|p| async { self.read(&p.unwrap()).await.unwrap() });

        Ok(futures::future::join_all(results)
            .await
            .iter()
            .filter_map(|r| r.clone())
            .collect())
    }

    pub async fn delete(&self, id: &String) -> Result<()> {
        let path = self.path.join(format!("{}.json", id));

        self.rm(&path).await
    }

    pub async fn clear(&self) -> Result<()> {
        let results = glob(self.path.join("*.json").to_str().unwrap())?
            .map(|p| async { self.rm(&p.unwrap()).await.unwrap() });

        futures::future::join_all(results).await;

        Ok(())
    }

    async fn read(&self, path: &PathBuf) -> Result<Option<Item>> {
        match fs::read(path).await {
            Ok(s) => {
                let mut item: Item = serde_json::from_slice(&s)?;
                item.id = path.file_stem().unwrap().to_str().unwrap().to_string();

                Ok(Some(item))
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::NotFound {
                    Ok(None)
                } else {
                    Err(e.into())
                }
            }
        }
    }

    async fn rm(&self, path: &PathBuf) -> Result<()> {
        fs::remove_file(path).await.map_err(|e| e.into())
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Item {
    #[serde(skip_serializing)]
    pub id: String,
    pub received_at: DateTime<Utc>,
    pub local: Url,
    pub request: RequestMessage,
}

impl Item {
    pub fn new(received_at: DateTime<Utc>, local: Url, request: RequestMessage) -> Self {
        let mut generator = names::Generator::default();
        let id = generator.next().unwrap();

        Self {
            id,
            received_at,
            local,
            request,
        }
    }
}
