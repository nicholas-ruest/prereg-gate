use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use prereg_gate::{Verdict, evaluate};

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Evaluate evidence against a preregistered experiment.
    Check(CheckArgs),
}

#[derive(Debug, Args)]
struct CheckArgs {
    /// TOML preregistration contract.
    #[arg(long)]
    spec: PathBuf,
    /// CSV evidence with metric,value columns.
    #[arg(long)]
    evidence: PathBuf,
    /// Write the JSON receipt to this path instead of stdout.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Render indented JSON.
    #[arg(long)]
    pretty: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(verdict) => ExitCode::from(verdict.exit_code()),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<Verdict, Box<dyn std::error::Error>> {
    let Cli { command } = Cli::parse();
    match command {
        Command::Check(args) => {
            let spec = fs::read(&args.spec)?;
            let evidence = fs::read(&args.evidence)?;
            let receipt = evaluate(&spec, &evidence)?;
            let json = if args.pretty {
                serde_json::to_string_pretty(&receipt)?
            } else {
                serde_json::to_string(&receipt)?
            };

            if let Some(output) = args.output {
                fs::write(output, format!("{json}\n"))?;
            } else {
                println!("{json}");
            }
            Ok(receipt.verdict)
        }
    }
}
