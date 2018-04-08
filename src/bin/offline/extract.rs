use clap;
use offline;
use rla;
use std::io::{self, Write};
use std::path::Path;

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

pub fn one(args: &clap::ArgMatches) -> rla::Result<()> {
    let index_file = Path::new(args.value_of_os("index-file").unwrap());
    let log_file = Path::new(args.value_of_os("log").unwrap());

    let index = rla::Index::load(index_file)?;
    let log = offline::fs::load_maybe_compressed(log_file)?;

    let lines = load_lines(&log);

    let config = rla::extract::Config::default();

    let blocks = rla::extract::extract(&config, &index, &lines);

    let mut first = true;

    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    for block in &blocks {
        if !first {
            writeln!(&mut stdout, "---")?;
        }
        first = false;

        for &line in block {
            stdout.write_all(&line.sanitized)?;
            stdout.write_all(b"\n")?;
        }
    }

    Ok(())
}
