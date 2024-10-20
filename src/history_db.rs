use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use glob::glob;
use hookhub::RequestMessage;
use serde::{Deserialize, Serialize};
use tokio::{fs, io};

#[derive(Serialize, Deserialize, Default)]
pub struct Db {
    path: PathBuf,
}

type ItemId = String;

impl Db {
    pub fn new(path: &PathBuf) -> Result<Self> {
        match std::fs::create_dir(path) {
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

    pub async fn get(&self, id: ItemId) -> Result<Option<Item>> {
        let path = self.path.join(format!("{}.json", id));

        self.read(&path).await
    }

    pub async fn add(&self, item: &Item) -> Result<ItemId> {
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

    pub async fn delete(&self, id: ItemId) -> Result<()> {
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
    pub id: ItemId,
    pub received_at: DateTime<Utc>,
    pub request: RequestMessage,
}

impl Item {
    pub fn new(received_at: DateTime<Utc>, request: RequestMessage) -> Self {
        let mut generator = names::Generator::default();
        let id = generator.next().unwrap();

        Self {
            id,
            received_at,
            request,
        }
    }
}
