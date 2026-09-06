use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use vault_io::atomic::create_with_writer_if_absent;
use vault_io::path_safety::{metadata_is_link_or_reparse, validate_descendant, EntryKind};

use crate::{ObjectId, Sha256Digest};

const OBJECT_MAGIC: &[u8; 5] = b"AVOBJ";
const OBJECT_FORMAT_VERSION: u8 = 1;
const OBJECT_CODEC_ZLIB: u8 = 1;
const OBJECT_HEADER_LEN: usize = 16;

/// Maximum logical size of one independently addressed object.
pub const MAX_OBJECT_BYTES: usize = 16 * 1024 * 1024;
const MAX_ENCODED_OBJECT_BYTES: u64 = (MAX_OBJECT_BYTES + 1024 * 1024 + OBJECT_HEADER_LEN) as u64;

pub type ObjectStoreResult<T> = Result<T, ObjectStoreError>;

#[derive(Debug, thiserror::Error)]
pub enum ObjectStoreError {
    #[error("content-addressed object I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("content-addressed object atomic I/O failed: {0}")]
    VaultIo(#[from] vault_io::Error),
    #[error("object store root must be an existing ordinary directory: {0}")]
    InvalidObjectRoot(PathBuf),
    #[error("content-addressed object is too large: {size} bytes exceeds the {max}-byte limit")]
    ObjectTooLarge { size: u64, max: u64 },
    #[error(
        "encoded content-addressed object is too large: {size} bytes exceeds the {max}-byte limit"
    )]
    EncodedObjectTooLarge { size: u64, max: u64 },
    #[error("content-addressed object envelope is invalid: {0}")]
    InvalidEnvelope(&'static str),
    #[error("unsupported content-addressed object format version {0}")]
    UnsupportedFormatVersion(u8),
    #[error("unsupported content-addressed object codec {0}")]
    UnsupportedCodec(u8),
    #[error("content-addressed object length mismatch: expected {expected}, read {actual}")]
    ObjectLengthMismatch { expected: u64, actual: u64 },
    #[error("content-addressed object digest mismatch: path is {expected}, content is {actual}")]
    ObjectDigestMismatch {
        expected: ObjectId,
        actual: ObjectId,
    },
    #[error("content-addressed object {0} collides with different existing content")]
    ObjectContentCollision(ObjectId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectDisposition {
    Created,
    Reused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredObject {
    id: ObjectId,
    logical_size: u64,
    disposition: ObjectDisposition,
}

impl StoredObject {
    pub const fn id(&self) -> &ObjectId {
        &self.id
    }

    pub const fn logical_size(&self) -> u64 {
        self.logical_size
    }

    pub const fn disposition(&self) -> ObjectDisposition {
        self.disposition
    }
}

/// A write-once content-addressed object store rooted at a caller-selected `objects` directory.
#[derive(Debug, Clone)]
pub struct ObjectStore {
    objects_root: PathBuf,
    sha256_root: PathBuf,
}

impl ObjectStore {
    pub fn open(objects_root: impl Into<PathBuf>) -> ObjectStoreResult<Self> {
        let objects_root = objects_root.into();
        let metadata = fs::symlink_metadata(&objects_root)?;
        if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
            return Err(ObjectStoreError::InvalidObjectRoot(objects_root));
        }

        let sha256_root = objects_root.join("sha256");
        validate_descendant(
            &objects_root,
            &sha256_root,
            EntryKind::Directory,
            true,
            "content-addressed object namespace",
        )?;
        match fs::create_dir(&sha256_root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        validate_descendant(
            &objects_root,
            &sha256_root,
            EntryKind::Directory,
            false,
            "content-addressed object namespace",
        )?;

        Ok(Self {
            objects_root,
            sha256_root,
        })
    }

    pub fn objects_root(&self) -> &Path {
        &self.objects_root
    }

    pub fn object_path(&self, id: &ObjectId) -> PathBuf {
        let digest = id.digest().to_string();
        self.sha256_root.join(&digest[..2]).join(&digest[2..])
    }

    pub fn put_bytes(&self, bytes: &[u8]) -> ObjectStoreResult<StoredObject> {
        require_object_size(bytes.len() as u64)?;
        let id = object_id(bytes);
        let path = self.object_path(&id);
        let fanout = path.parent().expect("object path has parent");
        validate_descendant(
            &self.objects_root,
            fanout,
            EntryKind::Directory,
            true,
            "content-addressed object fan-out",
        )?;
        match fs::create_dir(fanout) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        validate_descendant(
            &self.objects_root,
            fanout,
            EntryKind::Directory,
            false,
            "content-addressed object fan-out",
        )?;

        let exists = validate_descendant(
            &self.objects_root,
            &path,
            EntryKind::File,
            true,
            "content-addressed object",
        )?;
        if exists {
            return self.reuse_existing(id, bytes);
        }
        let encoded = encode_object(bytes)?;
        match create_with_writer_if_absent(&path, |file| {
            file.write_all(&encoded)?;
            Ok(())
        }) {
            Ok(()) => {}
            Err(error) if error.atomic_write_not_committed() => {
                let appeared = validate_descendant(
                    &self.objects_root,
                    &path,
                    EntryKind::File,
                    true,
                    "content-addressed object",
                )?;
                if appeared {
                    return self.reuse_existing(id, bytes);
                }
                return Err(error.into());
            }
            Err(error) => return Err(error.into()),
        }

        Ok(StoredObject {
            id,
            logical_size: bytes.len() as u64,
            disposition: ObjectDisposition::Created,
        })
    }

    pub fn read(&self, id: &ObjectId) -> ObjectStoreResult<Vec<u8>> {
        let path = self.object_path(id);
        validate_descendant(
            &self.objects_root,
            &path,
            EntryKind::File,
            false,
            "content-addressed object",
        )?;
        let bytes = decode_object(&read_encoded_object(&path)?)?;
        let actual = object_id(&bytes);
        if actual != *id {
            return Err(ObjectStoreError::ObjectDigestMismatch {
                expected: *id,
                actual,
            });
        }
        Ok(bytes)
    }

    fn reuse_existing(&self, id: ObjectId, expected: &[u8]) -> ObjectStoreResult<StoredObject> {
        let existing = self.read(&id)?;
        if existing != expected {
            return Err(ObjectStoreError::ObjectContentCollision(id));
        }
        Ok(StoredObject {
            id,
            logical_size: expected.len() as u64,
            disposition: ObjectDisposition::Reused,
        })
    }
}

fn object_id(bytes: &[u8]) -> ObjectId {
    ObjectId::sha256(Sha256Digest::from_bytes(Sha256::digest(bytes).into()))
}

fn read_encoded_object(path: &Path) -> ObjectStoreResult<Vec<u8>> {
    let file = fs::File::open(path)?;
    let size = file.metadata()?.len();
    if size > MAX_ENCODED_OBJECT_BYTES {
        return Err(ObjectStoreError::EncodedObjectTooLarge {
            size,
            max: MAX_ENCODED_OBJECT_BYTES,
        });
    }

    let mut encoded = Vec::with_capacity(size as usize);
    file.take(MAX_ENCODED_OBJECT_BYTES + 1)
        .read_to_end(&mut encoded)?;
    if encoded.len() as u64 > MAX_ENCODED_OBJECT_BYTES {
        return Err(ObjectStoreError::EncodedObjectTooLarge {
            size: encoded.len() as u64,
            max: MAX_ENCODED_OBJECT_BYTES,
        });
    }
    Ok(encoded)
}

fn encode_object(bytes: &[u8]) -> ObjectStoreResult<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    let compressed = encoder.finish()?;

    let mut encoded = Vec::with_capacity(OBJECT_HEADER_LEN + compressed.len());
    encoded.extend_from_slice(OBJECT_MAGIC);
    encoded.push(OBJECT_FORMAT_VERSION);
    encoded.push(OBJECT_CODEC_ZLIB);
    encoded.push(0);
    encoded.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&compressed);
    Ok(encoded)
}

fn decode_object(encoded: &[u8]) -> ObjectStoreResult<Vec<u8>> {
    if encoded.len() < OBJECT_HEADER_LEN {
        return Err(ObjectStoreError::InvalidEnvelope("header is truncated"));
    }
    if &encoded[..OBJECT_MAGIC.len()] != OBJECT_MAGIC {
        return Err(ObjectStoreError::InvalidEnvelope("magic does not match"));
    }
    if encoded[5] != OBJECT_FORMAT_VERSION {
        return Err(ObjectStoreError::UnsupportedFormatVersion(encoded[5]));
    }
    if encoded[6] != OBJECT_CODEC_ZLIB {
        return Err(ObjectStoreError::UnsupportedCodec(encoded[6]));
    }
    if encoded[7] != 0 {
        return Err(ObjectStoreError::InvalidEnvelope(
            "reserved header byte is non-zero",
        ));
    }

    let expected = u64::from_le_bytes(
        encoded[8..OBJECT_HEADER_LEN]
            .try_into()
            .expect("object length header has fixed width"),
    );
    require_object_size(expected)?;
    let mut decoder = ZlibDecoder::new(&encoded[OBJECT_HEADER_LEN..]);
    let mut bytes = Vec::new();
    decoder
        .by_ref()
        .take(expected.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let actual = bytes.len() as u64;
    if actual != expected {
        return Err(ObjectStoreError::ObjectLengthMismatch { expected, actual });
    }
    Ok(bytes)
}

fn require_object_size(size: u64) -> ObjectStoreResult<()> {
    let max = MAX_OBJECT_BYTES as u64;
    if size > max {
        return Err(ObjectStoreError::ObjectTooLarge { size, max });
    }
    Ok(())
}
