use std::collections::HashMap;

use anyhow::{anyhow, Result};
use log::info;
use serde::{Deserialize, Serialize};
use tokio::fs;
use url::Url;

use crate::{ProfilesCommands, ROOT_PATH};

pub struct Profiles {
    profiles: HashMap<String, Profile>,
}

impl Profiles {
    pub fn new() -> Result<Self> {
        let path = ROOT_PATH.join("profiles.json");

        match std::fs::read(path) {
            Ok(data) => {
                let profiles: HashMap<String, Profile> = serde_json::from_slice(&data)?;

                Ok(Self { profiles })
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(Self {
                        profiles: HashMap::new(),
                    })
                } else {
                    Err(e.into())
                }
            }
        }
    }

    pub fn get(&self, name: &String) -> Option<Profile> {
        self.profiles.get(name).map(|p| p.to_owned())
    }

    pub async fn add(&mut self, name: &String, profile: Profile) -> Result<()> {
        let path = ROOT_PATH.join("profiles.json");
        let mut profiles = self.profiles.clone();

        match profiles.insert(name.clone(), profile) {
            Some(_) => Err(anyhow!("profile {} already exists", name)),
            None => {
                let data = serde_json::to_vec_pretty(&profiles)?;
                fs::write(path, data).await?;

                self.profiles = profiles;
                Ok(())
            }
        }
    }

    pub async fn delete(&mut self, name: &String) -> Result<()> {
        let path = ROOT_PATH.join("profiles.json");
        let mut profiles = self.profiles.clone();

        match profiles.remove_entry(name) {
            Some(_) => {
                let data = serde_json::to_vec_pretty(&profiles)?;
                fs::write(path, data).await?;

                self.profiles = profiles;
                Ok(())
            }
            None => Err(anyhow!("profile {} doesn't exist", name)),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Profile {
    pub remote: Url,
    pub secret: String,
    pub local: Url,
}

impl Profile {
    pub fn prepare(&self) -> Result<Profile> {
        if self.remote.scheme() != "ws" && self.remote.scheme() != "wss" {
            return Err(anyhow::anyhow!("remote must use ws or wss scheme"));
        }

        if self.local.scheme() != "http" && self.local.scheme() != "https" {
            return Err(anyhow::anyhow!("local must use http or https scheme"));
        }

        let mut remote = self.remote.clone();
        let mut local = self.local.clone();

        remote.set_path("/__hookhub__/");
        local.set_path("/");

        Ok(Profile {
            remote,
            secret: self.secret.clone(),
            local,
        })
    }
}

pub async fn handle(command: ProfilesCommands) -> Result<()> {
    match command {
        ProfilesCommands::List => handle_list().await,
        ProfilesCommands::Delete { name } => handle_delete(name).await,
        ProfilesCommands::Add {
            name,
            remote,
            secret,
            local,
        } => handle_add(name, remote, secret, local).await,
    }
}

async fn handle_list() -> Result<()> {
    for (name, profile) in Profiles::new()?.profiles.iter() {
        info!(
            "[{}] Remote: {} Local: {}",
            name, profile.remote, profile.local
        );
    }

    Ok(())
}

async fn handle_delete(name: String) -> Result<()> {
    Profiles::new()?.delete(&name).await?;

    info!("Profile {} deleted", name);

    Ok(())
}

async fn handle_add(name: String, remote: Url, secret: String, local: Url) -> Result<()> {
    Profiles::new()?
        .add(
            &name,
            Profile {
                remote,
                secret,
                local,
            },
        )
        .await?;

    info!("Profile {} added", name);

    Ok(())
}
