use core::str;
use std::{
    fs::{create_dir_all, File},
    io::{self, BufRead, BufReader, BufWriter, Read, SeekFrom},
    path::{Path, PathBuf},
};

use clap::Parser;
use indexmap::IndexMap;
use log::{debug, error, info, LevelFilter};
use serde_pickle::DeOptions;
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
    /// extract files to the given path (default: the current working
    /// directory).
    #[clap(short, long, default_value = "./")]
    path: PathBuf,
    /// will make any missing directories in the given extraction path.
    #[clap(short, long)]
    mkdir: bool,
    /// Options that most users don't need, but might allow working with
    /// unsupported or damaged archives.
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
    #[clap(flatten)]
    overwrites: Option<ExtractOptions>,
}

#[derive(clap::Args, Clone, Debug, Copy)]
#[group(requires_all = &["offset", "key"])]
struct ExtractOptions {
    /// ignore the archive header and use this exact offset.
    #[arg(short, long)]
    #[arg(required = false)]
    key: u64,
    /// ignore the archive header and use this exact offset.
    #[arg(short, long)]
    #[arg(required = false)]
    offset: u64,
}

#[derive(clap::ValueEnum, Clone, Debug, Copy, PartialEq, Eq)]
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
    #[error("Could not create output file: {0}")]
    InvalidOutFile(std::io::Error),
    #[error("Could not determine archive version")]
    UnknownArchive,
    #[error("Could not decompress zlib archive index: {0}")]
    InvalidZLIBIndex(zune_inflate::errors::InflateDecodeErrors),
    #[error("Could not parse archive index: {0}")]
    InvalidIndex(serde_pickle::Error),
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

    if args.display.tree {
        todo!();
    } else if args.display.list {
        for input_file in &args.files {
            if let Err(e) = list_archive(
                input_file,
                args.advanced.overwrites,
                args.advanced.force,
            ) {
                error!("{e} ({})'", input_file.to_string_lossy());
                continue;
            }
        }
    } else {
        extract_archives(args);
    }
}

fn list_archive(
    input_file: &Path,
    overwrites: Option<ExtractOptions>,
    overwrite_version: Option<RPAVersion>,
) -> Result<(), UnrpaError> {
    let file = File::open(input_file).map_err(UnrpaError::FileRead)?;
    let mut reader = BufReader::new(file);

    let options = determine_options(
        input_file,
        overwrites,
        overwrite_version,
        &mut reader,
    )?;

    let mut index = parse_index(&mut reader, options)?;
    index.sort_keys();
    println!("{}:", input_file.to_string_lossy());
    for k in index.keys() {
        println!("\t{}", k.0.to_string_lossy())
    }

    Ok(())
}

fn extract_archives(args: Args) {
    if args.mkdir {
        if let Err(e) = std::fs::create_dir_all(&args.path)
            .map_err(UnrpaError::InvalidOutDir)
        {
            log::error!("{e}");
            std::process::exit(1);
        }
    }

    if !args.path.is_dir() {
        log::error!("Could not find output directory");
        std::process::exit(1);
    }

    for input_file in &args.files {
        let input_str = input_file.to_string_lossy();
        debug!("Extracting {input_str}");
        if let Err(e) = extract_archive(
            input_file,
            args.advanced.force,
            args.advanced.overwrites,
            &args.path,
        ) {
            error!("{e} ({input_str})'");
            continue;
        }
    }
}

fn determine_options(
    input_path: &Path,
    overwrites: Option<ExtractOptions>,
    overwrite_version: Option<RPAVersion>,
    reader: &mut impl BufRead,
) -> Result<HeaderInfo, UnrpaError> {
    let extension = input_path.extension().and_then(|a| a.to_str());
    let mut header = read_header(reader, extension, overwrite_version)?;

    if let Some(overwrites) = overwrites {
        header.key = Some(overwrites.key);
        header.offset = overwrites.offset;
    }
    Ok(header)
}

fn extract_archive(
    input_file: &Path,
    overwrite_version: Option<RPAVersion>,
    overwrites: Option<ExtractOptions>,
    out_path: &Path,
) -> Result<(), UnrpaError> {
    let file = File::open(input_file).map_err(UnrpaError::FileRead)?;
    let mut reader = BufReader::new(file);

    let options = determine_options(
        input_file,
        overwrites,
        overwrite_version,
        &mut reader,
    )?;

    let index = parse_index(&mut reader, options)?;
    let total_files = index.len();

    for (idx, (k, v)) in index.into_iter().enumerate() {
        let out_file = out_path.join(&k.0);
        if let Some(p) = out_file.parent() {
            create_dir_all(p).map_err(UnrpaError::InvalidOutDir)?;
        }
        info!(
            "[{:04.2}%] {:>3}",
            (idx as f64 / total_files as f64) * 100.0,
            out_file.to_string_lossy()
        );
        extract_file(&out_file, v, &mut reader)?;
    }

    debug!("Index contains {total_files} files");

    Ok(())
}

fn extract_file<R: BufRead + std::io::Seek>(
    out_file: &Path,
    idx_entry: Vec<IndexEntry>,
    archive: &mut R,
) -> Result<(), UnrpaError> {
    let output_file =
        File::create(out_file).map_err(UnrpaError::InvalidOutFile)?;
    let mut writer = BufWriter::new(output_file);
    for entry in idx_entry {
        archive
            .seek(SeekFrom::Start(entry.offset))
            .map_err(UnrpaError::FileRead)?;
        let mut archive = archive.take(entry.length);
        io::copy(&mut archive, &mut writer).map_err(UnrpaError::FileRead)?;
    }

    Ok(())
}

fn parse_index<R: BufRead + std::io::Seek>(
    reader: &mut R,
    options: HeaderInfo,
) -> Result<IndexMap<IndexKey, Vec<IndexEntry>>, UnrpaError> {
    reader
        .seek(SeekFrom::Start(options.offset))
        .map_err(UnrpaError::FileRead)?;

    let mut index = vec![];
    reader
        .read_to_end(&mut index)
        .map_err(UnrpaError::FileRead)?;

    let decompressed = zune_inflate::DeflateDecoder::new(&index)
        .decode_zlib()
        .map_err(UnrpaError::InvalidZLIBIndex)?;
    drop(index);

    let raw_index: IndexMap<IndexKey, Vec<GenericIndexEntry>> =
        serde_pickle::from_slice(&decompressed, DeOptions::new())
            .map_err(UnrpaError::InvalidIndex)?;
    drop(decompressed);

    let mut normalized_index = IndexMap::new();
    for (index_key, index_value) in raw_index {
        let mut vals = vec![];
        for val in index_value {
            let mut value: IndexEntry = val.into();
            if let Some(key) = options.key {
                value.offset ^= key;
                value.length ^= key;
            }
            vals.push(value);
        }
        normalized_index.insert(index_key, vals);
    }
    Ok(normalized_index)
}

#[derive(Debug, serde::Deserialize, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct IndexKey(PathBuf);

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
struct IndexEntry {
    offset: u64,
    length: u64,
    #[serde(with = "serde_bytes")]
    start: Vec<u8>,
}

impl From<GenericIndexEntry> for IndexEntry {
    fn from(value: GenericIndexEntry) -> Self {
        match value {
            GenericIndexEntry::SimpleIndexPart(a, b) => IndexEntry {
                offset: a,
                length: b,
                start: vec![],
            },
            GenericIndexEntry::ComplexIndexPart(a, b, vec) => IndexEntry {
                offset: a,
                length: b,
                start: vec,
            },
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum GenericIndexEntry {
    SimpleIndexPart(u64, u64),
    ComplexIndexPart(u64, u64, #[serde(with = "serde_bytes")] Vec<u8>),
}

struct HeaderInfo {
    offset: u64,
    key: Option<u64>,
}

fn read_header(
    reader: &mut impl BufRead,
    extension: Option<&str>,
    ov_version: Option<RPAVersion>,
) -> Result<HeaderInfo, UnrpaError> {
    if extension == Some("rpi")
        && ov_version.map_or(true, |a| a == RPAVersion::RPA1)
    {
        return Ok(HeaderInfo {
            key: None,
            offset: 0,
        });
    }
    let mut header = [0; 7];
    reader
        .read_exact(&mut header)
        .map_err(UnrpaError::FileRead)?;

    debug!(
        "Found header: {}",
        str::from_utf8(&header).unwrap_or("Unknown")
    );

    let version = match ov_version {
        Some(version) => version,
        None => match &header {
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
        },
    };

    let make_u64 = |str| {
        u64::from_str_radix(str, 16).map_err(|_| UnrpaError::UnknownArchive)
    };

    let (offset, key) = match version {
        RPAVersion::RPA1 => unreachable!("RPA1 does not check headers"),
        RPAVersion::RPA2 => {
            let mut line = String::new();
            reader.read_line(&mut line).map_err(UnrpaError::FileRead)?;
            (make_u64(line.trim())?, None)
        }
        RPAVersion::RPA3 | RPAVersion::RPA32 | RPAVersion::RPA40 => {
            let mut line = String::new();
            reader.read_line(&mut line).map_err(UnrpaError::FileRead)?;
            let (offset, key) = line
                .trim()
                .split_once(' ')
                .ok_or(UnrpaError::UnknownArchive)?;
            let offset = make_u64(offset)?;
            let key = make_u64(key)?;
            (offset, Some(key))
        }
        RPAVersion::ALT1 => {
            let mut line = String::new();
            reader.read_line(&mut line).map_err(UnrpaError::FileRead)?;
            let (key, offset) = line
                .trim()
                .split_once(' ')
                .ok_or(UnrpaError::UnknownArchive)?;

            let key = make_u64(key)? ^ 0xDABE8DF0;
            let offset = make_u64(offset)?;
            (offset, Some(key))
        }
        RPAVersion::ZiX12A => todo!(),
        RPAVersion::ZiX12B => todo!(),
    };
    debug!("Found offset: {offset}");
    if let Some(key) = key {
        debug!("Found key: {key}");
    }

    Ok(HeaderInfo { key, offset })
}
