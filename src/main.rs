use clap::Parser;
use itertools::Itertools;
use upload::upload;

mod upload;

#[derive(Parser, Clone)]
#[command(author, version, about, long_about = None)]
pub struct UploadOpts {
    /// Paths to crate files.
    pub crate_paths: Vec<String>,
    #[arg(short, long)]
    pub token: Option<String>,
    /// The registry name in the cargo config (see https://doc.rust-lang.org/cargo/reference/registries.html)
    #[arg(short, long)]
    pub index: Option<String>,
    #[arg(short, long, default_value_t = true)]
    pub keep_going: bool,
    #[arg(short, long)]
    pub dry_run: bool,
    #[arg(short, long)]
    pub registry: Option<String>,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info")
    }
    env_logger::init();

    // Skip upload subcommand keyword for using with cargo.
    let args = std::env::args().collect_vec();
    let args = if args
        .get(1)
        .map(|a| a == "upload")
        .unwrap_or(false)
    {
        UploadOpts::parse_from(&args[1..])
    } else {
        UploadOpts::parse()
    };
    upload(args).await.unwrap();
}
