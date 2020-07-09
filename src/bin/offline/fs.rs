use crate::rla;
use brotli;
use failure::ResultExt;
use percent_encoding::{AsciiSet, CONTROLS};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

const BROTLI_BUFFER: usize = 4096;

// Defaults from the Python implementation
const BROTLI_QUALITY: u32 = 11;
const BROTLI_LGWIN: u32 = 22;

/// The set of characters which cannot be used in a [filename on Windows][windows].
///
/// [windows]: https://docs.microsoft.com/en-us/windows/desktop/fileio/naming-a-file#naming-conventions
const FILENAME_ENCODE_SET: AsciiSet = CONTROLS
    .add(b'<')
    .add(b'>')
    .add(b':')
    .add(b'"')
    .add(b'/')
    .add(b'\\')
    .add(b'|')
    .add(b'?')
    .add(b'*');

pub fn save_compressed(out: &Path, data: &[u8]) -> rla::Result<()> {
    let mut writer = brotli::CompressorWriter::new(
        fs::File::create(out).with_context(|_| format!("save_compressed: {:?}", out.to_owned()))?,
        BROTLI_BUFFER,
        BROTLI_QUALITY,
        BROTLI_LGWIN,
    );

    writer.write_all(data)?;

    Ok(())
}

pub fn load_compressed(inp: &Path) -> rla::Result<Vec<u8>> {
    let mut reader = brotli::Decompressor::new(fs::File::open(inp)?, BROTLI_BUFFER);

    let mut buf = vec![];
    reader.read_to_end(&mut buf)?;

    Ok(buf)
}

pub fn load_maybe_compressed(inp: &Path) -> rla::Result<Vec<u8>> {
    if inp.extension().map_or(false, |e| e == "brotli") {
        load_compressed(inp)
    } else {
        let mut buf = vec![];
        fs::File::open(inp)?.read_to_end(&mut buf)?;
        Ok(buf)
    }
}

pub(crate) fn encode_path(path: &str) -> String {
    percent_encoding::percent_encode(path.as_bytes(), &FILENAME_ENCODE_SET).collect::<String>()
}
