// Implementation of Vault trait that actually stores files to disk.

use crate::database::Database;
use crate::types::*;
use fuse_mt::FilesystemMT;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::{Arc, Mutex};

// Vault should just use the database for storage, the main thing
// local vault needs to do is manage the metadata consistency: When
// creating a file, we need to update the parent directory to add this
// file; return an error if user tries to remove a non-empty
// directory; creating a file requires checking if the parent
// directory exists, etc. I would use the key-value store (set, get)
// to storage metadata, eg, each directory is a vector of file names
// under it. Then we opening or deleting files, we can just "get" the
// directory's content, remove or add to it and "set" it back.
#[derive(Debug)]
pub struct LocalVault {
    database: Arc<Database>,
}

impl LocalVault {
    pub fn new(database: Arc<Database>) -> VaultResult<LocalVault> {
        todo!()
    }
}

impl Vault for LocalVault {
    fn read(&self, file: &Path, offset: u64) -> VaultResult<Vec<u8>> {
        todo!()
    }
    fn write(&self, file: &Path, offset: u64, data: Vec<u8>) -> VaultResult<u64> {
        todo!()
    }
    fn open(&self, file: &Path, mode: OpenOptions) -> VaultResult<()> {
        todo!()
    }
    fn close(&self, file: &Path) -> VaultResult<()> {
        todo!()
    }
    fn mkdir(&self, parent: &Path, name: String) -> VaultResult<()> {
        todo!()
    }
    fn delete(&self, file: &Path) -> VaultResult<()> {
        todo!()
    }
    fn rmdir(&self, dir: &Path) -> VaultResult<()> {
        todo!()
    }
    fn readdir(&self, dir: &Path) -> VaultResult<Vec<String>> {
        todo!()
    }
}
