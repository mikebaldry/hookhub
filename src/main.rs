use std::{fs, io, path::PathBuf, sync::LazyLock};

use anyhow::Result;
use env_logger::Env;

use clap::{Parser, Subcommand};
use url::Url;

mod client;
mod history;
mod profiles;
mod server;

pub static ROOT_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let home = homedir::my_home().unwrap().unwrap();
    let path = home.join(".hookhub");

    match fs::create_dir(&path) {
        Ok(_) => path,
        Err(e) => {
            if e.kind() == io::ErrorKind::AlreadyExists {
                path
            } else {
                panic!("{}", e);
            }
        }
    }
});

/// Hookhub client
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Connect to a remote server and relay requests to a local server
    Connect {
        /// The profile to use, if not the default profile
        #[arg(long, default_value = "default")]
        profile: String,
    },
    /// Start a server to relay requests to clients
    Server {
        /// The address to listen on
        #[arg(long, env = "HOOKHUB_BIND_ADDR")]
        bind_addr: String,
        /// The secret that clients will need to connect
        #[arg(long, env = "HOOKHUB_SECRET")]
        secret: String,
    },
    /// Manage and replay previously received requests
    History {
        #[command(subcommand)]
        command: HistoryCommands,
    },
    /// Manage profile profiles
    Profiles {
        #[command(subcommand)]
        command: ProfilesCommands,
    },
}

#[derive(Subcommand)]
enum HistoryCommands {
    /// List previously received requests
    List,
    /// Delete a previously received request
    Delete {
        /// Identifier of the request
        id: String,
    },
    /// Clear all previously received requests
    Clear,
    /// Replay a previously received request
    Replay {
        /// Identifier of the request
        id: String,
    },
}

#[derive(Subcommand)]
enum ProfilesCommands {
    /// List profiles
    List,
    /// Delete a profile
    Delete {
        /// Name of the profile
        name: String,
    },
    /// Add a profile
    Add {
        /// Name of the profile
        #[arg(default_value = "default")]
        name: String,
        /// Remote origin to connect to, to receive requests from (e.g. wss://some.public/)
        #[arg(long)]
        remote: Url,
        /// Remote origins secret to authenticate with
        #[arg(long)]
        secret: String,
        /// Local origin to forward requests to (e.g. https://localhost:3000/)
        #[arg(long)]
        local: Url,
    },
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    match args.command {
        Commands::Connect { profile } => client::handle_connect(profile).await,
        Commands::Server { bind_addr, secret } => server::handle(bind_addr, secret).await,
        Commands::History { command } => history::handle(command).await,
        Commands::Profiles { command } => profiles::handle(command).await,
    }
}
