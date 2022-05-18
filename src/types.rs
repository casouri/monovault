use std::boxed::Box;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
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
    SqliteError(rusqlite::Error),
    MarshallError(serde_json::Error),
    IOError(std::io::Error),
}

impl From<rusqlite::Error> for VaultError {
    fn from(err: rusqlite::Error) -> Self {
        VaultError::SqliteError(err)
    }
}

impl From<serde_json::Error> for VaultError {
    fn from(err: serde_json::Error) -> Self {
        VaultError::MarshallError(err)
    }
}

impl From<std::io::Error> for VaultError {
    fn from(err: std::io::Error) -> Self {
        VaultError::IOError(err)
    }
}

pub trait Vault {
    fn read(&self, file: &Path, offset: u64) -> VaultResult<Vec<u8>>;
    fn write(&self, file: &Path, offset: u64, data: Vec<u8>) -> VaultResult<u64>;
    /// We only care about read, write and create flag in `mode`.
    /// There is no permission checking so we don't need to return a
    /// file descriptor.
    fn open(&self, file: &Path, mode: OpenOptions) -> VaultResult<()>;
    fn close(&self, file: &Path) -> VaultResult<()>;
    fn delete(&self, file: &Path) -> VaultResult<()>;
    fn mkdir(&self, parent: &Path, name: String) -> VaultResult<()>;
    fn readdir(&self, dir: &Path) -> VaultResult<Vec<String>>;
    fn rmdir(&self, dir: &Path) -> VaultResult<()>;
}
