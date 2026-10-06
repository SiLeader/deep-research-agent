use clap::Parser;
use tracing_subscriber::EnvFilter;

mod config;

#[derive(Debug, Parser)]
struct Args {
    #[arg(
        long,
        help = "Path to the configuration file",
        default_value = "/etc/deep-research-agent/config.toml"
    )]
    config: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let config = match config::Config::from_file(&args.config) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };
}
