// Implementation of Vault trait that actually stores files to disk.

use crate::database::Database;
use crate::types::*;
use log::{debug, info};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering::SeqCst},
    Arc, Mutex,
};

/// Local vault delegates metadata work to the database, and mainly
/// works on locating the "data file" for each file, and reading and
/// writing data files.
#[derive(Debug)]
pub struct LocalVault {
    /// Name of this vault.
    name: String,
    /// Database for metadata.
    database: Arc<Mutex<Database>>,
    /// Maps inode to file handlers.
    fd_map: Mutex<HashMap<Inode, Arc<Mutex<File>>>>,
    /// Counts the number of references to each file, when ref count
    /// of that file reaches 0, the file handler can be closed, and
    /// the file can be deleted from disk (if requested).
    ref_count: Mutex<HashMap<Inode, u64>>,
    /// The next allocated inode is next_inode + 1.
    next_inode: AtomicU64,
}

impl LocalVault {
    /// `name` is the name of the vault, also the directory name of
    /// the vault root.
    pub fn new(name: &str, database: Arc<Mutex<Database>>) -> VaultResult<LocalVault> {
        let next_inode = { database.lock().unwrap().largest_inode() };
        info!("vault {} next_inode={}", name, next_inode);
        Ok(LocalVault {
            name: name.to_string(),
            database,
            fd_map: Mutex::new(HashMap::new()),
            ref_count: Mutex::new(HashMap::new()),
            next_inode: AtomicU64::new(next_inode),
        })
    }

    /// Return a new inode.
    fn new_inode(&self) -> Inode {
        self.next_inode
            .fetch_update(SeqCst, SeqCst, |inode| Some(inode + 1))
            .unwrap();
        self.next_inode.load(SeqCst)
    }

    /// Get the path to where the content of `file` is stored.
    /// Basically `db_path/vault_name-inode`.
    fn compose_path(&self, file: Inode) -> PathBuf {
        self.database
            .lock()
            .unwrap()
            .path()
            .join(format!("{}-{}", self.name(), file.to_string()))
    }

    /// Open and get the file handler for `file`. `file` is created if
    /// not already exists.
    fn get_file(&self, file: Inode) -> VaultResult<Arc<Mutex<File>>> {
        let mut map = self.fd_map.lock().unwrap();
        match map.get(&file) {
            Some(fd) => Ok(Arc::clone(fd)),
            None => {
                info!("get_file, path={:?}", self.compose_path(file));
                let fd = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(self.compose_path(file))?;
                let fdref = Arc::new(Mutex::new(fd));
                map.insert(file, Arc::clone(&fdref));
                Ok(fdref)
            }
        }
    }

    fn check_is_regular_file(&self, file: Inode) -> VaultResult<()> {
        let kind = self.database.lock().unwrap().attr(file)?.kind;
        match kind {
            VaultFileType::File => Ok(()),
            VaultFileType::Directory => Err(VaultError::IsDirectory(file)),
        }
    }

    /// Check if the corresponding data file for `file` exists on disk.
    fn check_data_file_exists(&self, file: Inode) -> VaultResult<()> {
        let path = self.compose_path(file);
        if path.exists() {
            Ok(())
        } else {
            Err(VaultError::FileNotExist(file))
        }
    }

    /// Increment ref count of `file`.
    fn incf_ref_count(&self, file: Inode) -> VaultResult<u64> {
        let mut map = self.ref_count.lock().unwrap();
        let count = match map.get(&file) {
            Some(&count) => count,
            None => 0,
        };
        if count == u64::MAX {
            Err(VaultError::U64Overflow(file))
        } else {
            map.insert(file, count + 1);
            Ok(count + 1)
        }
    }

    /// Decrement ref count of `file`.
    fn decf_ref_count(&self, file: Inode) -> VaultResult<u64> {
        let mut map = self.ref_count.lock().unwrap();
        let count = match map.get(&file) {
            Some(&count) => count,
            None => 0,
        };
        if count == 0 {
            Err(VaultError::U64Underflow(file))
        } else {
            map.insert(file, count - 1);
            Ok(count - 1)
        }
    }

    /// Return the ref count of `file`.
    fn ref_count(&self, file: Inode) -> u64 {
        match self.ref_count.lock().unwrap().get(&file) {
            Some(&count) => count,
            None => 0,
        }
    }
}

impl Vault for LocalVault {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn attr(&self, file: Inode) -> VaultResult<FileInfo> {
        debug!("attr(file={})", file);
        let entry = self.database.lock().unwrap().attr(file)?;
        let size = match entry.kind {
            VaultFileType::File => {
                let meta = std::fs::metadata(self.compose_path(file))?;
                meta.len()
            }
            VaultFileType::Directory => 1,
        };
        Ok(FileInfo {
            inode: entry.inode,
            name: entry.name,
            kind: entry.kind,
            size,
        })
    }

    fn read(&self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>> {
        info!("read(file={}, offset={}, size={})", file, offset, size);
        self.check_is_regular_file(file)?;
        self.check_data_file_exists(file)?;
        let lck = self.get_file(file)?;
        let mut file = lck.lock().unwrap();
        let mut buf = vec![0; size as usize];
        file.seek(SeekFrom::Start(offset as u64))?;
        // Read exact SIZE bytes, if not enough, read to EOF but don't error.
        match file.read_exact(&mut buf) {
            Ok(()) => Ok(buf),
            Err(err) => {
                if err.kind() == std::io::ErrorKind::UnexpectedEof {
                    file.read_to_end(&mut buf)?;
                    Ok(buf)
                } else {
                    Err(VaultError::IOError(err))
                }
            }
        }
    }

    fn write(&self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32> {
        info!("write(file={}, offset={})", file, offset);
        self.check_is_regular_file(file)?;
        self.check_data_file_exists(file)?;
        let lck = self.get_file(file)?;
        let mut file = lck.lock().unwrap();
        Ok(file.write(data)? as u32)
    }

    fn create(&self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        info!("create(parent={}, name={}, kind={:?})", parent, name, kind);
        let inode = self.new_inode();
        // In fuse semantics (and thus vault's) create also open the
        // file. We need to call get_file to ensure the data file is
        // created.
        self.get_file(inode)?;
        // NOTE: Make sure we create data file before creating
        // metadata, to ensure consistency.
        self.database
            .lock()
            .unwrap()
            .add_file(parent, inode, name, kind)?;
        self.incf_ref_count(inode)?;
        info!("created {}", inode);
        Ok(inode)
    }

    fn open(&self, file: Inode, mode: &mut OpenOptions) -> VaultResult<()> {
        info!("open(file={})", file);
        self.check_is_regular_file(file)?;
        self.check_data_file_exists(file)?;
        self.incf_ref_count(file)?;
        Ok(())
    }

    fn close(&self, file: Inode) -> VaultResult<()> {
        info!("close(file={})", file);
        self.check_is_regular_file(file)?;
        self.check_data_file_exists(file)?;
        let count = self.decf_ref_count(file)?;
        if count == 0 {
            let mut map = self.fd_map.lock().unwrap();
            map.remove(&file);
        }
        Ok(())
    }

    fn delete(&self, file: Inode) -> VaultResult<()> {
        info!("delete(file={})", file);
        // Database will check for nonempty directory for us.
        self.database.lock().unwrap().remove_file(file)?;
        // NOTE: Make sure we remove metadata before removing data
        // file, to ensure consistency.
        let kind = self.database.lock().unwrap().attr(file)?.kind;
        match kind {
            VaultFileType::File => {
                self.check_data_file_exists(file)?;
                if self.ref_count(file) == 0 {
                    std::fs::remove_file(self.compose_path(file))?;
                    // TODO: delete all pending files on exit.
                }
            }
            VaultFileType::Directory => (),
        }
        Ok(())
    }

    fn readdir(&self, dir: Inode) -> VaultResult<Vec<DirEntry>> {
        info!("readdir(dir={})", dir);
        self.database.lock().unwrap().readdir(dir)
    }
}
