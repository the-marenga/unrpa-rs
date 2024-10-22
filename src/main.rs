use core::str;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    ops::ControlFlow,
    path::{Path, PathBuf},
};

use clap::Parser;
use log::{debug, error, LevelFilter};
use thiserror::Error;

/// unrpa is a tool to extract files from Ren'Py archives (.rpa).
///
/// This program is free software: you can redistribute it and/or modify
/// it under the terms of the GNU General Public License as published by
/// the Free Software Foundation, either version 3 of the License, or
/// (at your option) any later version.
///
/// This program is distributed in the hope that it will be useful,
/// but WITHOUT ANY WARRANTY; without even the implied warranty of
/// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
/// GNU General Public License for more details.
///
/// You should have received a copy of the GNU General Public License
/// along with this program.  If not, see <http://www.gnu.org/licenses/>.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[clap(short, long)]
    /// explain what is being done, increase value for more verbosity
    verbose: bool,
    #[clap(short, long)]
    /// no non-essential output.
    silent: bool,

    #[clap(flatten)]
    display: ArgsDisplay,
    /// extract files to the given path (default: the current working directory).
    #[clap(short, long, default_value = "./")]
    path: PathBuf,
    /// will make any missing directories in the given extraction path.
    #[clap(short, long)]
    mkdir: bool,
    /// Options that most users don't need, but might allow working with unsupported or damaged archives.
    #[clap(flatten)]
    advanced: ArgsAdvanced,
    #[clap(num_args = 1.., value_delimiter = ' ', required=true)]
    files: Vec<PathBuf>,
}

#[derive(clap::Args, Copy, Clone, Debug)]
#[group(multiple = false)]
struct ArgsDisplay {
    /// list the contents of the archive(s) in a flat list.
    #[arg(short, long)]
    list: bool,
    /// list the contents of the archive(s) in a tree view
    #[arg(short, long)]
    tree: bool,
}

#[derive(clap::Args, Clone, Debug)]
struct ArgsAdvanced {
    /// try to continue extraction when something goes wrong.
    #[arg(short, long)]
    continue_on_error: bool,
    /// ignore the archive header and assume this exact version
    #[arg(short, long, value_enum, ignore_case = true)]
    force: Option<RPAVersion>,
    /// ignore the archive header and use this exact offset.
    #[arg(short, long)]
    offset: Option<u64>,
    /// ignore the archive header and use this exact key.
    #[arg(short, long)]
    key: Option<i32>,
}

#[derive(clap::ValueEnum, Clone, Debug, Copy)]
enum RPAVersion {
    #[clap(name = "RPA-1.0")]
    RPA1,
    #[clap(name = "RPA-2.0")]
    RPA2,
    #[clap(name = "RPA-3.0")]
    RPA3,
    #[clap(name = "ALT-1.0")]
    ALT1,
    #[clap(name = "ZiX-12A")]
    ZiX12A,
    #[clap(name = "ZiX-12B")]
    ZiX12B,
    #[clap(name = "RPA-3.2")]
    RPA32,
    #[clap(name = "RPA-4.0")]
    RPA40,
}

#[derive(Error, Debug)]
enum UnrpaError {
    #[error("Could not read file: {0}")]
    FileRead(std::io::Error),
    #[error("Could not create output directory: {0}")]
    InvalidOutDir(std::io::Error),
    #[error("Could not determine archive version")]
    UnknownArchive,
}

fn main() {
    let args = Args::parse();
    let log_level = if args.silent {
        LevelFilter::Off
    } else if args.verbose {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };

    env_logger::builder().filter_level(log_level).init();

    if args.display.tree || args.display.list {
        todo!();
    }

    if args.mkdir {
        if let Err(e) = std::fs::create_dir_all(&args.path).map_err(UnrpaError::InvalidOutDir) {
            log::error!("{e}");
            std::process::exit(1);
        }
    }

    if !args.path.is_dir() {
        panic!("Could not find output directory");
    }

    for input_file in &args.files {
        let input_str = input_file.to_string_lossy();
        debug!("Extracting {input_str}");
        if let Err(e) = extract_archive(input_file, args.advanced.force) {
            error!("{e} ({input_str})'");
            continue;
        }
    }
}

fn extract_archive(input_file: &Path, version: Option<RPAVersion>) -> Result<(), UnrpaError> {
    let extension = input_file.extension().and_then(|a| a.to_str());
    let file = File::open(input_file).map_err(UnrpaError::FileRead)?;
    let mut reader = BufReader::new(file);
    let version = match version {
        Some(version) => version,
        None => determine_version(&mut reader, extension)?,
    };

    Ok(())
}

fn determine_version(
    reader: &mut impl BufRead,
    extension: Option<&str>,
) -> Result<RPAVersion, UnrpaError> {
    if extension == Some("rpi") {
        return Ok(RPAVersion::RPA1);
    }
    let mut header = [0; 7];
    reader
        .read_exact(&mut header)
        .map_err(UnrpaError::FileRead)?;

    debug!(
        "Found header: {}",
        str::from_utf8(&header).unwrap_or("Unknown")
    );
    Ok(match header.as_slice() {
        b"ALT-1.0" => RPAVersion::ALT1,
        b"RPA-2.0" => RPAVersion::RPA2,
        b"RPA-3.0" => RPAVersion::RPA3,
        b"RPA-3.2" => RPAVersion::RPA32,
        b"RPA-4.0" => RPAVersion::RPA40,
        b"ZiX-12A" => RPAVersion::ZiX12A,
        b"ZiX-12B" => RPAVersion::ZiX12B,
        _ => {
            return Err(UnrpaError::UnknownArchive);
        }
    })
}
