use anyhow::Result;
use clap::{Parser, Subcommand};

mod client;
mod server;

use client::run_cli_client;
use server::run_cli_server;

#[derive(Parser)]
#[command(
    name = "wireless-display",
    version = "1.0",
    about = "Use another PC as external monitor for your current PC"
)]
struct AppCli {
    #[command(subcommand)]
    command: AppCommands,
}

#[derive(Subcommand)]
enum AppCommands {
    #[command(about = "Run as server")]
    Server {
        #[arg(help = "Port to listen on", short, long, default_value_t = 8787)]
        port: u16,
        #[arg(help = "Password for authentication", long)]
        password: Option<String>,
    },

    #[command(about = "Run as client")]
    Client {
        #[arg(help = "Password for authentication", long)]
        password: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = AppCli::parse();

    match cli.command {
        AppCommands::Server { port, password } => run_cli_server(port, password).await?,
        AppCommands::Client { password } => run_cli_client(password).await?,
    }

    Ok(())
}
