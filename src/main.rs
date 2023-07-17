#![warn(clippy::pedantic)]

mod asciicast;
mod config;

use std::{
    fs,
    io::{BufReader, BufWriter},
    path::PathBuf,
};

use clap::Parser;
use color_eyre::{eyre::Context, Help};

use config::{Script, Settings};

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    let in_file = fs::File::open(cli.in_file).wrap_err("could not open input file")?;

    let mut script = Script::try_from_yaml(BufReader::new(in_file))
        .wrap_err("could not parse input file as Script")?;
    script.merge_settings(cli.settings);

    let out_file = fs::File::options()
        .write(true)
        .create_new(!cli.overwrite)
        .create(cli.overwrite)
        .truncate(true)
        .open(cli.out_file)
        .wrap_err("could not create/open output file")
        .suggestion("use `--overwrite` if you wish to replace an existing file")?;

    let cast = asciicast::File::try_from(script).wrap_err("error running script")?;
    cast.write(BufWriter::new(out_file))
        .wrap_err("could not write to output file")?;

    Ok(())
}

#[derive(Parser, Debug, Clone)]
#[command(version, author, about)]
struct Cli {
    #[command(flatten)]
    settings: Settings,

    /// Overwrite output file if it already exists
    #[arg(long)]
    overwrite: bool,

    /// Input file to create the asciicast file with
    in_file: PathBuf,

    /// Output asciicast file
    out_file: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cli() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
