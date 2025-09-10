use anyhow::Result;
use clap::{Parser, Subcommand};

mod client;
mod server;
mod shared;

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
        #[arg(help = "Capture frame rate", short, long, default_value_t = 60)]
        framerate: u32,
        #[arg(help = "Pairing code", short, long, default_value_t = String::from("hello"))]
        code: String,
        #[arg(help = "Password for authentication", long)]
        password: Option<String>,
    },

    #[command(about = "Run as client")]
    Client {
        #[arg(help = "Pairing code", short, long, default_value_t = String::from("hello"))]
        code: String,
        #[arg(help = "Password for authentication", long)]
        password: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = AppCli::parse();

    match cli.command {
        AppCommands::Server {
            port,
            framerate,
            code,
            password,
        } => run_cli_server(port, framerate, code, password).await?,
        AppCommands::Client { code, password } => run_cli_client(code, password).await?,
    }

    Ok(())
}
