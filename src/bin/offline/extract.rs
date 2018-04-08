use clap;
use log;
use offline;
use rla;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use walkdir::{self, WalkDir};
use std::time::Instant;
use std::time::Duration;

struct Line<'a> {
    original: &'a [u8],
    sanitized: Vec<u8>,
}

impl<'a> rla::index::IndexData for Line<'a> {
    fn sanitized(&self) -> &[u8] {
        &self.sanitized
    }
}

fn load_lines(log: &[u8]) -> Vec<Line> {
    rla::sanitize::split_lines(log).iter().map(|&line| Line {
        original: line,
        sanitized: rla::sanitize::clean(line)
    }).collect()
}

pub fn dir(args: &clap::ArgMatches) -> rla::Result<()> {
    let index_file = Path::new(args.value_of_os("index-file").unwrap());
    let src_dir = Path::new(args.value_of_os("source").unwrap());
    let dst_dir = Path::new(args.value_of_os("destination").unwrap());

    let config = rla::extract::Config::default();
    let index = rla::Index::load(index_file)?;


    for entry in walk_non_hidden_children(dst_dir) {
        let entry = entry?;

        if entry.file_type().is_dir() {
            continue;
        }

        fs::remove_file(entry.path())?;
    }

    let mut count = 0;
    let progress_every = Duration::from_secs(1);
    let mut last_print = Instant::now();

    for entry in walk_non_hidden_children(src_dir) {
        let entry = entry?;

        if entry.file_type().is_dir() {
            continue;
        }

        count += 1;

        let now = Instant::now();

        let level = if now - last_print >= progress_every {
            last_print = now;
            log::Level::Debug
        } else {
            log::Level::Trace
        };

        log!(level, "Extracting erros from {} [{}/?]...", entry.path().display(), count);

        let log = offline::fs::load_maybe_compressed(entry.path())?;
        let lines = load_lines(&log);
        let blocks = rla::extract::extract(&config, &index, &lines);

        let mut out_name = entry.file_name().to_owned();
        out_name.push(".err");

        write_blocks_to(io::BufWriter::new(fs::File::create(dst_dir.join(out_name))?), &blocks)?;
    }

    Ok(())
}

pub fn one(args: &clap::ArgMatches) -> rla::Result<()> {
    let index_file = Path::new(args.value_of_os("index-file").unwrap());
    let log_file = Path::new(args.value_of_os("log").unwrap());

    let config = rla::extract::Config::default();
    let index = rla::Index::load(index_file)?;

    let log = offline::fs::load_maybe_compressed(log_file)?;
    let lines = load_lines(&log);
    let blocks = rla::extract::extract(&config, &index, &lines);

    let stdout = io::stdout();
    write_blocks_to(stdout.lock(), &blocks)?;

    Ok(())
}

fn write_blocks_to<W: Write>(mut w: W, blocks: &[Vec<&Line>]) -> rla::Result<()> {
    let mut first = true;

    for block in blocks {
        if !first {
            writeln!(w, "---")?;
        }
        first = false;

        for &line in block {
            w.write_all(&line.sanitized)?;
            w.write_all(b"\n")?;
        }
    }

    Ok(())
}

fn walk_non_hidden_children(root: &Path) -> Box<Iterator<Item=walkdir::Result<walkdir::DirEntry>>> {
    fn not_hidden(entry: &walkdir::DirEntry) -> bool {
        !entry.file_name().to_str().map(|s| s.starts_with('.')).unwrap_or(false)
    }

    Box::new(WalkDir::new(root).min_depth(1).max_depth(1).into_iter().filter_entry(not_hidden))
}
