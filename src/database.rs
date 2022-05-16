use crate::types::*;
use serde::{de::DeserializeOwned, Serialize};
use std::path::Path;

#[derive(Debug)]
pub struct Database {
    root: Path,
}

impl Database {
    // Read and write are used for storing file data.
    pub fn read(path: &Path, offset: u64) -> VaultResult<Vec<u8>> {
        todo!()
    }
    pub fn write(path: &Path, offset: u64, data: Vec<u8>) -> VaultResult<u64> {
        todo!()
    }
    // Set and get are used for storing metadata, like directory,
    // version, etc.
    pub fn set<T: Serialize>(path: &Path, value: T) -> VaultResult<()> {
        todo!()
    }
    pub fn get<T: DeserializeOwned>(path: &Path) -> VaultResult<T> {
        todo!()
    }
}
