// Implementation of Vault trait that actually stores files to disk.

use crate::database::Database;
use crate::types::*;
use fuse_mt::FilesystemMT;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

// Vault should just use the database to implement everything. Databse
// allows you to store files very easily, the main thing local_vault
// needs to do is manage the directories: When creating a file, we
// need to update the parent directory to add this file. I would use
// the key-value store (set, get) to implement directories, where each
// directory is a vector of file names under it. Then we opening or
// deleting files, we can just "get" the directory's content, remove
// or add to it and "set" it back.
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
    fn open(&self, file: &Path, mode: OpenMode) -> VaultResult<()> {
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
}
