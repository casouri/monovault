use std::boxed::Box;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub type VaultName = String;
pub type VaultAddress = String;

pub type VaultResult<T> = std::result::Result<T, VaultError>;

pub struct Config {
    my_address: VaultAddress,
    peers: HashMap<VaultName, VaultAddress>,
    mount_point: String,
    local_vault_name: VaultName,
}

pub enum VaultError {
    FileNotExist(String),
    NoWriteAccess(String),
    NetworkError(Box<dyn std::error::Error>),
    Unknown(Box<dyn std::error::Error>),
    WriteConflict(String, u64, u64),
}

pub enum OpenMode {
    Read,
    ReadWrite,
    CreateReadWrite,
}

pub trait Vault {
    fn read(&self, file: &Path, offset: u64) -> VaultResult<Vec<u8>>;
    fn write(&self, file: &Path, offset: u64, data: Vec<u8>) -> VaultResult<u64>;
    fn open(&self, file: &Path, mode: OpenMode) -> VaultResult<()>;
    fn close(&self, file: &Path) -> VaultResult<()>;
    fn mkdir(&self, parent: &Path, name: String) -> VaultResult<()>;
    fn delete(&self, file: &Path) -> VaultResult<()>;
}
