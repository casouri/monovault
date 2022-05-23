use crate::database::Database;
use crate::types::*;
use std::sync::{Arc, Mutex};

pub struct CachingVault {
    database: Arc<Mutex<Database>>,
    name: String,
    remote: Box<dyn Vault>,
}

impl Vault for CachingVault {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn attr(&self, file: Inode) -> VaultResult<FileInfo> {
        todo!()
    }

    fn read(&self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>> {
        todo!()
    }

    fn open(&self, file: Inode, mode: OpenMode) -> VaultResult<()> {
        todo!()
    }

    fn setup(&self) -> VaultResult<()> {
        todo!()
    }

    fn write(&self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32> {
        todo!()
    }

    fn close(&self, file: Inode) -> VaultResult<()> {
        todo!()
    }

    fn create(&self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        todo!()
    }

    fn delete(&self, file: Inode) -> VaultResult<()> {
        todo!()
    }

    fn readdir(&self, dir: Inode) -> VaultResult<Vec<DirEntry>> {
        todo!()
    }

    fn tear_down(&self) -> VaultResult<()> {
        todo!()
    }
}
