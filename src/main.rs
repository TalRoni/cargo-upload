use clap::Parser;
use itertools::Itertools;
use upload::upload;

mod upload;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct UploadOpts {
    /// Path to specific crate file or folder with crate files.
    #[arg(short, long)]
    pub crates_path: String,
    #[arg(short, long)]
    pub token: Option<String>,
    /// The registry name in the cargo config (see https://doc.rust-lang.org/cargo/reference/registries.html)
    #[arg(short, long)]
    pub index: Option<String>,
    #[arg(short, long)]
    pub keep_going: bool,
    #[arg(short, long)]
    pub dry_run: bool,
    #[arg(short, long)]
    pub registry: Option<String>,
}

fn main() {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info")
    }
    env_logger::init();

    // Skip upload subcommand keyword for using with cargo.
    let args = std::env::args().collect_vec();
    let args = if args
        .get(1)
        .and_then(|a| Some(a == "upload"))
        .unwrap_or(false)
    {
        UploadOpts::parse_from(&args[1..])
    } else {
        UploadOpts::parse()
    };
    upload(args).unwrap();
}
