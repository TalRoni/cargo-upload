use upload::upload;
use clap::Parser;

mod upload;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct UploadOpts {
    #[arg(short, long)]
    pub crate_path: String,
    #[arg(short, long)]
    pub token: Option<String>,
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
    env_logger::init();
    let args = UploadOpts::parse();
    upload(args).unwrap();
}
