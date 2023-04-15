use crate::{Index, Result};
use atomicwrites::{AtomicFile, OverwriteBehavior};
use std::fs::File;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub enum IndexStorage {
    FileSystem(FileSystemStorage),
}

impl IndexStorage {
    pub fn new(path: &str) -> Result<Self> {
        Ok(IndexStorage::FileSystem(FileSystemStorage {
            path: path.into(),
        }))
    }

    pub(super) fn read(&self) -> Result<Option<Index>> {
        match self {
            IndexStorage::FileSystem(fs) => fs.read(),
        }
    }

    pub(super) fn write(&self, index: &Index) -> Result<()> {
        match self {
            IndexStorage::FileSystem(fs) => fs.write(index),
        }
    }
}

impl FromStr for IndexStorage {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        IndexStorage::new(s)
    }
}

impl std::fmt::Display for IndexStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IndexStorage::FileSystem(fs) => write!(f, "{}", fs.path.display()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileSystemStorage {
    path: PathBuf,
}

impl FileSystemStorage {
    fn read(&self) -> Result<Option<Index>> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        Ok(Some(Index::deserialize(&mut file)?))
    }

    fn write(&self, index: &Index) -> Result<()> {
        let file = AtomicFile::new(&self.path, OverwriteBehavior::AllowOverwrite);
        file.write(|inner| index.serialize(inner))?;
        Ok(())
    }
}
