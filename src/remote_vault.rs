// Basically a gRPC client that makes requests to remote vault servers.

use crate::database::Database;
use crate::types::*;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug)]
pub struct RemoteVault {
    database: Arc<Database>,
}

impl Vault for RemoteVault {
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
