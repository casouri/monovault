use serde::{Deserialize, Serialize};
use std::boxed::Box;
use std::collections::HashMap;
use std::time;

pub type VaultName = String;
pub type VaultAddress = String;
pub type Inode = u64;

pub type VaultResult<T> = std::result::Result<T, VaultError>;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// The address our vault server listens on.
    pub my_address: VaultAddress,
    /// A map of peer name to addresses. Addresses should include
    /// address scheme (http://).
    pub peers: HashMap<VaultName, VaultAddress>,
    /// Mount point of the file system.
    pub mount_point: String,
    /// Path to the directory that stores the database.
    pub db_path: String,
    /// Name of the local vault.
    pub local_vault_name: VaultName,
    /// If true, cache remote files locally.
    pub caching: bool,
    /// If false, don't run a vault server that shares the local vault
    /// with peers.
    pub share_local_vault: bool,
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
    RemoteError(Box<dyn std::error::Error>),
    WriteConflict(Inode, u64, u64),
    SqliteError(rusqlite::Error),
    SystemTimeError(time::SystemTimeError),
    IOError(std::io::Error),
    RpcError(tonic::transport::Error),
}

impl From<rusqlite::Error> for VaultError {
    fn from(err: rusqlite::Error) -> Self {
        VaultError::SqliteError(err)
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
pub trait Vault: Send {
    /// Return the name of the vault.
    fn name(&self) -> String;
    fn tear_down(&mut self) -> VaultResult<()> {
        Ok(())
    }
    fn attr(&mut self, file: Inode) -> VaultResult<FileInfo>;
    /// Read `file` from `offset`, reads `size` bytes. If there aren't
    /// enough bytes to read, read to EOF.
    fn read(&mut self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>>;
    /// Write `data` into `file` at `offset`.
    fn write(&mut self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32>;
    /// Create a file or directory under `parent` with `name` and open
    /// it. Return its inode.
    fn create(&mut self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode>;
    /// Open `file`. `mod` is currently unused. `file` should be a regular file.
    fn open(&mut self, file: Inode, mode: OpenMode) -> VaultResult<()>;
    /// Close `file`. `file` should be a regular file.
    fn close(&mut self, file: Inode) -> VaultResult<()>;
    /// Delete `file`. `file` can a regular file or a directory.
    fn delete(&mut self, file: Inode) -> VaultResult<()>;
    /// List directory entries of `dir`. The listing includes "." and
    /// "..", but if `dir` is vault root, ".." is not included.
    fn readdir(&mut self, dir: Inode) -> VaultResult<Vec<FileInfo>>;
}
