use serde::{Deserialize, Serialize};
use std::boxed::Box;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::Path;

pub type VaultName = String;
pub type VaultAddress = String;
pub type Inode = u64;

pub type VaultResult<T> = std::result::Result<T, VaultError>;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub my_address: VaultAddress,
    pub peers: HashMap<VaultName, VaultAddress>,
    pub mount_point: String,
    pub db_path: String,
    pub local_vault_name: VaultName,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum VaultFileType {
    File,
    Directory,
}

#[derive(Debug)]
pub struct DirEntry {
    pub inode: Inode,
    pub name: String,
    pub kind: VaultFileType,
}

#[derive(Debug)]
pub enum VaultError {
    NoCorrespondingVault(Inode),
    U64Overflow(String),
    U64Underflow(String),
    FileNotExist(String),
    InvalidAction(String),
    NotDirectory(String),
    DeleteNonEmptyDirectory(String),
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

/// Inodes start from 1024, 0-1023 are reserved for vaults.
pub trait Vault {
    /// Return the name of the vault.
    fn name(&self) -> String;
    fn attr(&self, file: Inode) -> VaultResult<DirEntry>;
    /// Read `file` from `offset`, reads `size` bytes. If there aren't
    /// enough bytes to read, read to EOF.
    fn read(&self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>>;
    /// Write `data` into `file` at `offset`.
    fn write(&self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32>;
    /// Create a file or directory under `parent` with `name` and open
    /// it. Return its inode.
    fn create(&self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode>;
    /// Open `file`.
    fn open(&self, file: Inode, mode: &mut OpenOptions) -> VaultResult<()>;
    /// Close `file`.
    fn close(&self, file: Inode) -> VaultResult<()>;
    /// Delete `file`.
    fn delete(&self, file: Inode) -> VaultResult<()>;
    /// Read each entry in `dir`, and return them.
    fn readdir(&self, dir: Inode) -> VaultResult<Vec<DirEntry>>;
}
