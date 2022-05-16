// Implementation of Vault trait that actually stores files to disk.

use crate::database::Database;
use crate::types::*;
use fuse_mt::FilesystemMT;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct LocalVault {
    database: Arc<Database>,
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
}
