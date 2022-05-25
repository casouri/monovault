use serde::{Deserialize, Serialize};
use std::boxed::Box;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::Path;
use std::time;

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

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub inode: Inode,
    pub name: String,
    pub kind: VaultFileType,
    pub size: u64,
    pub atime: u64,
    pub mtime: u64,
    pub version: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum OpenMode {
    R,
    RW,
}

#[derive(Debug)]
pub enum VaultError {
    FileNameTooLong(String),
    NoCorrespondingVault(Inode),
    U64Overflow(u64),
    U64Underflow(u64),
    FileNotExist(Inode),
    NotDirectory(Inode),
    IsDirectory(Inode),
    DirectoryNotEmpty(Inode),
    NetworkError(Box<dyn std::error::Error>),
    Unknown(Box<dyn std::error::Error>),
    WriteConflict(Inode, u64, u64),
    SqliteError(rusqlite::Error),
    MarshallError(serde_json::Error),
    SystemTimeError(time::SystemTimeError),
    IOError(std::io::Error),
    RpcError(tonic::transport::Error),
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

impl From<time::SystemTimeError> for VaultError {
    fn from(err: time::SystemTimeError) -> Self {
        VaultError::SystemTimeError(err)
    }
}

impl From<tonic::transport::Error> for VaultError {
    fn from(err: tonic::transport::Error) -> Self {
        VaultError::RpcError(err)
    }
}

/// A generic vault, can be either a local vault or a remote vault.
pub trait Vault {
    /// Return the name of the vault.
    fn name(&self) -> String;
    fn setup(&self) -> VaultResult<()> {
        Ok(())
    }
    fn tear_down(&self) -> VaultResult<()> {
        Ok(())
    }
    fn attr(&self, file: Inode) -> VaultResult<FileInfo>;
    /// Read `file` from `offset`, reads `size` bytes. If there aren't
    /// enough bytes to read, read to EOF.
    fn read(&self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>>;
    /// Write `data` into `file` at `offset`.
    fn write(&self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32>;
    /// Create a file or directory under `parent` with `name` and open
    /// it. Return its inode.
    fn create(&self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode>;
    /// Open `file`. `mod` is currently unused. `file` should be a regular file.
    fn open(&self, file: Inode, mode: OpenMode) -> VaultResult<()>;
    /// Close `file`. `file` should be a regular file.
    fn close(&self, file: Inode) -> VaultResult<()>;
    /// Delete `file`. `file` can a regular file or a directory.
    fn delete(&self, file: Inode) -> VaultResult<()>;
    /// List directory entries of `dir`. The listing includes "." and
    /// "..", but if `dir` is vault root, ".." is not included.
    fn readdir(&self, dir: Inode) -> VaultResult<Vec<FileInfo>>;
}
