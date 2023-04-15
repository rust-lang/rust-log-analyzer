use super::Result;
use std::slice;

mod storage;
mod table;

pub use self::storage::IndexStorage;

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

    pub fn save(&self, storage: &IndexStorage) -> Result<()> {
        debug!("Saving index to '{storage}'...");
        storage.write(self)?;
        debug!("Index saved.");
        Ok(())
    }

    pub fn load(storage: &IndexStorage) -> Result<Index> {
        Index::load_or_create_internal(storage, false)
    }

    pub fn load_or_create(storage: &IndexStorage) -> Result<Index> {
        Index::load_or_create_internal(storage, true)
    }

    fn load_or_create_internal(storage: &IndexStorage, create: bool) -> Result<Index> {
        info!("Loading index...");
        let index = if let Some(index) = storage.read()? {
            index
        } else {
            if create {
                info!("Index missing, initializing new index...");
                Index::default()
            } else {
                anyhow::bail!("missing index, aborting");
            }
        };

        info!("Index ready ({} keys).", index.internal.len());

        Ok(index)
    }

    fn deserialize(reader: &mut dyn std::io::Read) -> Result<Self> {
        Ok(bincode::deserialize_from(reader)?)
    }

    fn serialize(
        &self,
        writer: &mut dyn std::io::Write,
    ) -> std::result::Result<(), bincode::Error> {
        bincode::serialize_into(writer, self)?;
        Ok(())
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

    #[allow(clippy::complexity)]
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
