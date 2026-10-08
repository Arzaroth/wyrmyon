use clap::Parser;

#[derive(Parser)]
#[command(version, about)]
struct Cli {}

pub fn main() {
    Cli::parse();
}
