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
    fn name(&self) -> String { todo!() } 
    fn attr(&self, file: Inode) -> VaultResult<FileInfo> { todo!() } 
    fn read(&self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>> { todo!() }
    fn write(&self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32> { todo!() }
    fn create(&self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> { todo!() }
    fn open(&self, file: Inode, mode: &mut OpenOptions) -> VaultResult<()> { todo!() }
    fn close(&self, file: Inode) -> VaultResult<()> { todo!() }
    fn delete(&self, file: Inode) -> VaultResult<()> { todo!() }
    fn readdir(&self, dir: Inode) -> VaultResult<Vec<DirEntry>> { todo!() }
}
