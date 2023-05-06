use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    #[arg(short, long)]
    pub pair_name: Option<String>,

    #[arg(short, long)]
    pub exchange_ident: Option<Vec<String>>,

    /// Number of times to greet
    #[arg(short, long, default_value = "http://[::1]:50051")]
    pub server_addr: String,
}
