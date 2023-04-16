use crate::offline;
use crate::rla;

use rla::index::IndexStorage;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use walkdir::{self, WalkDir};

pub fn learn(
    ci: &dyn rla::ci::CiPlatform,
    index_file: &IndexStorage,
    inputs: &[PathBuf],
    multiplier: u32,
) -> rla::Result<()> {
    let mut index = rla::Index::load_or_create(index_file)?;

    let progress_every = Duration::from_secs(1);
    let mut last_print = Instant::now();

    for (count, input) in inputs
        .iter()
        .flat_map(|i| WalkDir::new(i).into_iter().filter_entry(not_hidden))
        .enumerate()
    {
        let input = input?;
        if input.file_type().is_dir() {
            continue;
        }

        let now = Instant::now();

        if now - last_print >= progress_every {
            last_print = now;
            debug!("Learning from {} [{}/?]...", input.path().display(), count);
        } else {
            trace!("Learning from {} [{}/?]...", input.path().display(), count);
        }

        let data = offline::fs::load_maybe_compressed(input.path())?;

        for line in rla::sanitize::split_lines(&data) {
            index.learn(
                &rla::index::Sanitized(rla::sanitize::clean(ci, line)),
                multiplier,
            );
        }
    }

    index.save(index_file)?;

    Ok(())
}

fn not_hidden(entry: &walkdir::DirEntry) -> bool {
    !entry
        .file_name()
        .to_str()
        .map(|s| s.starts_with('.'))
        .unwrap_or(false)
}
