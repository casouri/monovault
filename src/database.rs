use crate::types::*;
use serde::{de::DeserializeOwned, Serialize};
use std::boxed::Box;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

// ROOT of the data base is basically /db, where / is the mounting
// point of the file system. I would hash paths and store data under
// hashed names. Eg, hash /a/b to 1220338343, and store the content in
// /db/data/1220338343. This way we avoid messing with directories.
// Set and get basically gives you a key-value store that local_vault
// can use for storing meta data, like directories and anything else.
// The simplest implementation is just serialize the value to string,
// and store as a file. Eg, set("/a/b", stuff) -> hash /a/b to
// 1220338343, and store serialized stuff under /db/kv/1220338343.

/// Database provides object storage and key-value storage.
#[derive(Debug)]
pub struct Database {
    root: PathBuf,
    hasher: Box<DefaultHasher>,
}

impl Database {
    pub fn new(root: &Path) -> VaultResult<Database> {
        Ok(Database {
            root: root.to_path_buf(),
            hasher: Box::new(DefaultHasher::new()),
        })
    }
    pub fn open(path: &Path) -> VaultResult<()> {
        todo!()
    }
    pub fn file_exists(path: &Path) -> VaultResult<bool> {
        todo!()
    }
    // Read and write are used for storing file data.
    pub fn read(path: &Path, offset: u64) -> VaultResult<Vec<u8>> {
        todo!()
    }
    pub fn write(path: &Path, offset: u64, data: Vec<u8>) -> VaultResult<u64> {
        todo!()
    }
    pub fn delete(path: &Path) -> VaultResult<()> {
        todo!()
    }
    // Set and get are used for storing metadata, like directory,
    // version, etc. Setting None means delete the value.
    pub fn set<T: Serialize>(path: &Path, value: Option<T>) -> VaultResult<()> {
        todo!()
    }
    pub fn get<T: DeserializeOwned>(path: &Path) -> VaultResult<Option<T>> {
        todo!()
    }
}
