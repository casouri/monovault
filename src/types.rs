use crate::caching_remote::CachingVault;
use crate::local_vault::LocalVault;
use crate::remote_vault::RemoteVault;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time;

pub type VaultName = String;
pub type VaultAddress = String;
pub type Inode = u64;
pub type VaultRef = Arc<Mutex<GenericVault>>;
pub type VaultResult<T> = std::result::Result<T, VaultError>;

/// 100 network MB. Packets are split into packets on wire, this chunk
/// size limit is just for saving memory. (Once we implement chunked
/// read & write.)
pub const GRPC_DATA_CHUNK_SIZE: usize = 1000000 * 100;

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
    /// Whether allow disconnected delete.
    pub allow_disconnected_delete: bool,
    /// Whether to allow disconnected create.
    pub allow_disconnected_create: bool,
    /// Wait this long between each background synchronization to
    /// remote vaults.
    pub background_update_interval: u8,
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
    // Errors that are returned from local and remote vault.
    FileNameTooLong(String),
    FileNotExist(Inode),
    NotDirectory(Inode),
    IsDirectory(Inode),
    DirectoryNotEmpty(Inode),
    FileAlreadyExist(Inode, String),
    // Error that are returned from remote vault.
    RpcError(String),
    RemoteError(String),
    // All errors below are squashed into a RemoteError if returned
    // from a remove vault. They are returned normally if from a local
    // vault.
    NoCorrespondingVault(Inode),
    WrongTypeOfVault(String),
    CannotFindVaultByName(String),
    U64Overflow(u64),
    U64Underflow(u64),
    WriteConflict(Inode, u64, u64),
    SqliteError(rusqlite::Error),
    SystemTimeError(time::SystemTimeError),
    IOError(std::io::Error),
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
        VaultError::RpcError(format!("{}", err))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum CompressedError {
    FileNameTooLong(String),
    FileNotExist(Inode),
    NotDirectory(Inode),
    IsDirectory(Inode),
    DirectoryNotEmpty(Inode),
    CannotFindVaultByName(String),
    FileAlreadyExist(Inode, String),
    Misc(String),
}

impl From<VaultError> for CompressedError {
    fn from(err: VaultError) -> Self {
        match err {
            VaultError::FileNameTooLong(name) => CompressedError::FileNameTooLong(name),
            VaultError::FileNotExist(inode) => CompressedError::FileNotExist(inode),
            VaultError::NotDirectory(inode) => CompressedError::NotDirectory(inode),
            VaultError::IsDirectory(inode) => CompressedError::IsDirectory(inode),
            VaultError::DirectoryNotEmpty(inode) => CompressedError::DirectoryNotEmpty(inode),
            VaultError::CannotFindVaultByName(name) => CompressedError::CannotFindVaultByName(name),
            VaultError::FileAlreadyExist(inode, name) => {
                CompressedError::FileAlreadyExist(inode, name)
            }

            VaultError::SqliteError(err) => CompressedError::Misc(format!("{}", err)),
            VaultError::NoCorrespondingVault(err) => CompressedError::Misc(format!("{}", err)),
            VaultError::U64Overflow(err) => CompressedError::Misc(format!("{}", err)),
            VaultError::U64Underflow(err) => CompressedError::Misc(format!("{}", err)),
            VaultError::RemoteError(err) => CompressedError::Misc(format!("{}", err)),
            VaultError::SystemTimeError(err) => CompressedError::Misc(format!("{}", err)),
            VaultError::IOError(err) => CompressedError::Misc(format!("{}", err)),
            VaultError::RpcError(err) => CompressedError::Misc(format!("{}", err)),
            VaultError::WrongTypeOfVault(expecting) => CompressedError::Misc(expecting),
            VaultError::WriteConflict(err0, err1, err2) => {
                CompressedError::Misc(format!("{}, {}, {}", err0, err1, err2))
            }
        }
    }
}

impl From<CompressedError> for VaultError {
    fn from(err: CompressedError) -> Self {
        match err {
            CompressedError::FileNameTooLong(name) => VaultError::FileNameTooLong(name),
            CompressedError::FileNotExist(inode) => VaultError::FileNotExist(inode),
            CompressedError::NotDirectory(inode) => VaultError::NotDirectory(inode),
            CompressedError::IsDirectory(inode) => VaultError::IsDirectory(inode),
            CompressedError::DirectoryNotEmpty(inode) => VaultError::DirectoryNotEmpty(inode),
            CompressedError::CannotFindVaultByName(name) => VaultError::CannotFindVaultByName(name),
            CompressedError::FileAlreadyExist(inode, name) => {
                VaultError::FileAlreadyExist(inode, name)
            }
            CompressedError::Misc(err) => VaultError::RemoteError(err),
        }
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

pub enum GenericVault {
    Local(LocalVault),
    Remote(RemoteVault),
    Caching(CachingVault),
}

pub fn unpack_to_caching(vault: &mut GenericVault) -> VaultResult<&mut CachingVault> {
    match vault {
        GenericVault::Caching(vault) => Ok(vault),
        _ => Err(VaultError::WrongTypeOfVault("caching".to_string())),
    }
}

pub fn unpack_to_local(vault: &mut GenericVault) -> VaultResult<&mut LocalVault> {
    match vault {
        GenericVault::Local(vault) => Ok(vault),
        _ => Err(VaultError::WrongTypeOfVault("local".to_string())),
    }
}

pub fn unpack_to_remote(vault: &mut GenericVault) -> VaultResult<&mut RemoteVault> {
    match vault {
        GenericVault::Remote(vault) => Ok(vault),
        _ => Err(VaultError::WrongTypeOfVault("remote".to_string())),
    }
}

impl Vault for GenericVault {
    fn name(&self) -> String {
        match self {
            GenericVault::Local(vault) => vault.name(),
            GenericVault::Remote(vault) => vault.name(),
            GenericVault::Caching(vault) => vault.name(),
        }
    }

    fn attr(&mut self, file: Inode) -> VaultResult<FileInfo> {
        match self {
            GenericVault::Local(vault) => vault.attr(file),
            GenericVault::Remote(vault) => vault.attr(file),
            GenericVault::Caching(vault) => vault.attr(file),
        }
    }

    fn read(&mut self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>> {
        match self {
            GenericVault::Local(vault) => vault.read(file, offset, size),
            GenericVault::Remote(vault) => vault.read(file, offset, size),
            GenericVault::Caching(vault) => vault.read(file, offset, size),
        }
    }

    fn write(&mut self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32> {
        match self {
            GenericVault::Local(vault) => vault.write(file, offset, data),
            GenericVault::Remote(vault) => vault.write(file, offset, data),
            GenericVault::Caching(vault) => vault.write(file, offset, data),
        }
    }

    fn create(&mut self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        match self {
            GenericVault::Local(vault) => vault.create(parent, name, kind),
            GenericVault::Remote(vault) => vault.create(parent, name, kind),
            GenericVault::Caching(vault) => vault.create(parent, name, kind),
        }
    }

    fn open(&mut self, file: Inode, mode: OpenMode) -> VaultResult<()> {
        match self {
            GenericVault::Local(vault) => vault.open(file, mode),
            GenericVault::Remote(vault) => vault.open(file, mode),
            GenericVault::Caching(vault) => vault.open(file, mode),
        }
    }

    fn close(&mut self, file: Inode) -> VaultResult<()> {
        match self {
            GenericVault::Local(vault) => vault.close(file),
            GenericVault::Remote(vault) => vault.close(file),
            GenericVault::Caching(vault) => vault.close(file),
        }
    }

    fn delete(&mut self, file: Inode) -> VaultResult<()> {
        match self {
            GenericVault::Local(vault) => vault.delete(file),
            GenericVault::Remote(vault) => vault.delete(file),
            GenericVault::Caching(vault) => vault.delete(file),
        }
    }

    fn readdir(&mut self, dir: Inode) -> VaultResult<Vec<FileInfo>> {
        match self {
            GenericVault::Local(vault) => vault.readdir(dir),
            GenericVault::Remote(vault) => vault.readdir(dir),
            GenericVault::Caching(vault) => vault.readdir(dir),
        }
    }
}
