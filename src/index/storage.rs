use crate::{Index, Result};
use anyhow::anyhow;
use atomicwrites::{AtomicFile, OverwriteBehavior};
use aws_sdk_s3::config::Region;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::Client as S3Client;
use std::fs::File;
use std::io::Cursor;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::runtime::Runtime;

#[derive(Debug, Clone)]
pub enum IndexStorage {
    FileSystem(FileSystemStorage),
    S3(Arc<S3Storage>),
}

impl IndexStorage {
    pub fn new(path: &str) -> Result<Self> {
        if let Some(s3_url) = path.strip_prefix("s3://") {
            let (bucket, key) = s3_url
                .split_once('/')
                .ok_or_else(|| anyhow!("invalid s3 url: {path}"))?;
            Ok(IndexStorage::S3(Arc::new(S3Storage::new(bucket, key)?)))
        } else {
            Ok(IndexStorage::FileSystem(FileSystemStorage {
                path: path.into(),
            }))
        }
    }

    pub(super) fn read(&self) -> Result<Option<Index>> {
        match self {
            IndexStorage::FileSystem(fs) => fs.read(),
            IndexStorage::S3(s3) => s3.read(),
        }
    }

    pub(super) fn write(&self, index: &Index) -> Result<()> {
        match self {
            IndexStorage::FileSystem(fs) => fs.write(index),
            IndexStorage::S3(s3) => s3.write(index),
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
            IndexStorage::S3(s3) => write!(f, "s3://{}/{}", s3.bucket, s3.key),
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

#[derive(Debug)]
pub struct S3Storage {
    runtime: Runtime,
    client: S3Client,
    bucket: String,
    key: String,
}

impl S3Storage {
    fn new(bucket: &str, key: &str) -> Result<Self> {
        let runtime = Runtime::new()?;

        let config = runtime.block_on(async {
            let global_config = aws_config::load_from_env().await;
            let global_s3 = S3Client::new(&global_config);

            let location = global_s3
                .get_bucket_location()
                .bucket(bucket)
                .send()
                .await?;
            let region = location
                .location_constraint()
                .map(|c| c.as_str())
                .unwrap_or("us-east-1")
                .to_string();

            info!("using S3 bucket {bucket} in region {region}");

            let regional_config = aws_config::from_env()
                .region(Region::new(region))
                .load()
                .await;

            Ok::<_, anyhow::Error>(regional_config)
        })?;
        let client = S3Client::new(&config);

        Ok(S3Storage {
            runtime,
            client,
            bucket: bucket.into(),
            key: key.into(),
        })
    }

    fn read(&self) -> Result<Option<Index>> {
        self.runtime.block_on(async {
            let result = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(&self.key)
                .send()
                .await;

            match result {
                Ok(response) => {
                    // FIXME: this buffers the downloaded data into memory before deserializing it,
                    // as I'm not aware of a way to convert from AsyncRead to Read.
                    let mut buf = Vec::new();
                    tokio::io::copy(&mut response.body.into_async_read(), &mut buf).await?;
                    Ok(Some(Index::deserialize(&mut Cursor::new(buf))?))
                }
                Err(err) => {
                    if let SdkError::ServiceError(service_err) = &err {
                        if let GetObjectError::NoSuchKey(_) = service_err.err() {
                            return Ok(None);
                        }
                    }
                    Err(err.into())
                }
            }
        })
    }

    fn write(&self, index: &Index) -> Result<()> {
        self.runtime.block_on(async {
            // FIXME: this buffers the serialized data into memory before sending it, as I'm not
            // aware of a way to convert from Write to AsyncWrite.
            let mut buf = Vec::new();
            index.serialize(&mut Cursor::new(&mut buf))?;

            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(&self.key)
                .body(buf.into())
                .send()
                .await?;

            Ok(())
        })
    }
}
