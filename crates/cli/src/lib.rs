use std::process::ExitCode;

use clap::Parser;

#[derive(Parser)]
#[command(version, about)]
struct Cli {}

pub fn main() -> ExitCode {
    Cli::parse();
    ExitCode::SUCCESS
}
