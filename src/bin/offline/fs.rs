use crate::rla;
use brotli;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

const BROTLI_BUFFER: usize = 4096;

// Defaults from the Python implementation
const BROTLI_QUALITY: u32 = 11;
const BROTLI_LGWIN: u32 = 22;

pub fn save_compressed(out: &Path, data: &[u8]) -> rla::Result<()> {
    let mut writer = brotli::CompressorWriter::new(
        fs::File::create(out)?,
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
