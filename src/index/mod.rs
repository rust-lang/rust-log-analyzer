use super::Result;
use atomicwrites;
use bincode;
use fnv;
use std;
use std::fs;
use std::path::Path;
use std::slice;

mod table;

pub trait IndexData {
    fn sanitized(&self) -> &[u8];
}

pub struct Sanitized<T: AsRef<[u8]>>(pub T);

impl<T: AsRef<[u8]>> IndexData for Sanitized<T> {
    fn sanitized(&self) -> &[u8] {
        self.0.as_ref()
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Index {
    internal: fnv::FnvHashMap<u32, u32>,
}

impl Index {
    pub fn learn<I: IndexData>(&mut self, data: &I, multiplier: u32) {
        let encoded = encode(data);

        for id in IdIter::new(&encoded) {
            let val = self.internal.entry(id).or_insert(0);
            *val = val.saturating_add(multiplier);
        }
    }

    pub fn scores<I: IndexData>(&self, data: &I) -> std::vec::IntoIter<u32> {
        let encoded = encode(data);

        IdIter::new(&encoded)
            .map(|id| self.internal.get(&id).cloned().unwrap_or(0))
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        debug!("Saving index to '{}'...", path.display());
        let file = atomicwrites::AtomicFile::new(path, atomicwrites::AllowOverwrite);
        file.write(|f| bincode::serialize_into(f, self))?;
        debug!("Index saved.");
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Index> {
        Index::load_or_create_internal(path, false)
    }

    pub fn load_or_create(path: &Path) -> Result<Index> {
        Index::load_or_create_internal(path, true)
    }

    fn load_or_create_internal(path: &Path, create: bool) -> Result<Index> {
        let index;

        if path.exists() || !create {
            info!("Loading index...");
            index = bincode::deserialize_from(fs::File::open(path)?)?;
        } else {
            info!("Initializing new index...");
            index = Index::default();
        };

        info!("Index ready ({} keys).", index.internal.len());

        Ok(index)
    }
}

pub fn encode<I: IndexData>(data: &I) -> Vec<u8> {
    data.sanitized()
        .iter()
        .map(|&b| table::ASCII_ID_MAP[b as usize])
        .filter(|&b| b != 0xFF)
        .collect()
}

pub fn decode(data: &[u8]) -> Vec<u8> {
    data.iter()
        .map(|&b| table::ID_ASCII_MAP[b as usize])
        .collect()
}

struct IdIter<'a> {
    windows: slice::Windows<'a, u8>,
}

impl<'a> IdIter<'a> {
    fn new(encoded_data: &'a [u8]) -> IdIter<'a> {
        IdIter {
            windows: encoded_data.windows(5),
        }
    }
}

impl<'a> Iterator for IdIter<'a> {
    type Item = u32;

    #[cfg_attr(feature = "cargo-clippy", allow(clippy_complexity))]
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        self.windows.next().map(|w| {
            0u32 + w[0] as u32
                + w[1] as u32 * 64
                + w[2] as u32 * 64 * 64
                + w[3] as u32 * 64 * 64 * 64
                + w[4] as u32 * 64 * 64 * 64 * 64
        })
    }
}
