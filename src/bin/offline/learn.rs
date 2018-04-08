use clap;
use log;
use offline;
use rla;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;
use walkdir::{self, WalkDir};

pub fn learn(args: &clap::ArgMatches) -> rla::Result<()> {
    let index_file = Path::new(args.value_of_os("index-file").unwrap());
    let multiplier: u32 = args.value_of("multiplier").unwrap().parse()?;
    let inputs = args.values_of_os("logs").unwrap();

    let mut index = rla::Index::load_or_create(index_file)?;

    let mut count = 0;
    let progress_every = Duration::from_secs(1);
    let mut last_print = Instant::now();

    for input in inputs.flat_map(|i| WalkDir::new(i).into_iter().filter_entry(not_hidden)) {
        let input = input?;
        if input.file_type().is_dir() {
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

        log!(level, "Learning from {} [{}/?]...", input.path().display(), count);

        let data = offline::fs::load_maybe_compressed(input.path())?;

        for line in rla::sanitize::split_lines(&data) {
            index.learn(&rla::index::Sanitized(rla::sanitize::clean(line)), multiplier);
        }
    }

    index.save(index_file)?;

    Ok(())
}

fn not_hidden(entry: &walkdir::DirEntry) -> bool {
    !entry.file_name().to_str().map(|s| s.starts_with('.')).unwrap_or(false)
}
