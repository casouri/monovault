// Implementation of Vault trait that actually stores files to disk.

use crate::database::Database;
use crate::types::*;
use log::info;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering::SeqCst},
    Arc, Mutex,
};

// Vault should just use the database for storage, the main thing
// local vault needs to do is manage the metadata consistency: When
// creating a file, we need to update the parent directory to add this
// file; return an error if user tries to remove a non-empty
// directory; creating a file requires checking if the parent
// directory exists, etc. I would use the key-value store (set, get)
// to storage metadata, eg, each directory is a vector of file names
// under it. Then we opening or deleting files, we can just "get" the
// directory's content, remove or add to it and "set" it back.
#[derive(Debug)]
pub struct LocalVault {
    /// Name of this vault.
    name: String,
    /// Database for meta information.
    database: Arc<Mutex<Database>>,
    /// Maps inode to file handlers.
    fd_map: Mutex<HashMap<Inode, Arc<Mutex<File>>>>,
    /// Counts the number of references to each file, when ref count
    /// of that file reaches 0, the file handler can be closed, and
    /// the file can be deleted (if requested).
    ref_count: Mutex<HashMap<Inode, u64>>,
    /// The next allocated inode is next_inode + 1.
    next_inode: AtomicU64,
}

impl LocalVault {
    pub fn new(name: &str, database: Arc<Mutex<Database>>) -> VaultResult<LocalVault> {
        let next_inode = { database.lock().unwrap().largest_inode() };
        Ok(LocalVault {
            name: name.to_string(),
            database,
            fd_map: Mutex::new(HashMap::new()),
            ref_count: Mutex::new(HashMap::new()),
            next_inode: AtomicU64::new(next_inode),
        })
    }

    fn new_inode(&self) -> Inode {
        self.next_inode
            .fetch_update(SeqCst, SeqCst, |inode| Some(inode + 1))
            .unwrap()
    }

    fn compose_path(&self, file: Inode) -> PathBuf {
        self.database
            .lock()
            .unwrap()
            .path()
            .join(format!("{}-{}", self.name(), file.to_string()))
    }

    // Get the file handler for FILE. FILE is created if not exists.
    fn get_file(&self, file: Inode) -> VaultResult<Arc<Mutex<File>>> {
        let mut map = self.fd_map.lock().unwrap();
        match map.get(&file) {
            Some(fd) => Ok(Arc::clone(fd)),
            None => {
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

    fn check_exists(&self, file: Inode) -> VaultResult<()> {
        let path = self.compose_path(file);
        if path.exists() {
            Ok(())
        } else {
            Err(VaultError::FileNotExist(file.to_string()))
        }
    }

    fn incf_ref_count(&self, file: Inode) -> VaultResult<u64> {
        let mut map = self.ref_count.lock().unwrap();
        let count = match map.get(&file) {
            Some(&count) => count,
            None => 0,
        };
        if count == u64::MAX {
            Err(VaultError::U64Overflow(file.to_string()))
        } else {
            map.insert(file, count + 1);
            Ok(count + 1)
        }
    }

    fn decf_ref_count(&self, file: Inode) -> VaultResult<u64> {
        let mut map = self.ref_count.lock().unwrap();
        let count = match map.get(&file) {
            Some(&count) => count,
            None => 0,
        };
        if count == 0 {
            Err(VaultError::U64Underflow(file.to_string()))
        } else {
            map.insert(file, count - 1);
            Ok(count - 1)
        }
    }

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

    fn attr(&self, file: Inode) -> VaultResult<DirEntry> {
        info!("attr(file={})", file);
        self.database.lock().unwrap().attr(file)
    }

    fn read(&self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>> {
        info!("read(file={}, offset={}, size={})", file, offset, size);
        let lck = self.get_file(file)?;
        let mut file = lck.lock().unwrap();
        let mut buf = vec![0; size as usize];
        file.seek(SeekFrom::Start(offset as u64))?;
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
        let lck = self.get_file(file)?;
        let mut file = lck.lock().unwrap();
        Ok(file.write(data)? as u32)
    }

    fn create(&self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        info!("create(parent={}, name={}, kind={:?})", parent, name, kind);
        let inode = self.new_inode();
        self.database
            .lock()
            .unwrap()
            .add_file(parent, inode, name, kind)?;
        self.incf_ref_count(inode)?;
        Ok(inode)
    }

    fn open(&self, file: Inode, mode: &mut OpenOptions) -> VaultResult<()> {
        info!("open(file={})", file);
        self.check_exists(file)?;
        self.incf_ref_count(file)?;
        Ok(())
    }

    fn close(&self, file: Inode) -> VaultResult<()> {
        info!("close(file={})", file);
        let _ = self.get_file(file)?;
        let count = self.decf_ref_count(file)?;
        if count == 0 {
            let mut map = self.fd_map.lock().unwrap();
            map.remove(&file);
        }
        Ok(())
    }

    fn delete(&self, file: Inode) -> VaultResult<()> {
        info!("delete(file={})", file);
        self.database.lock().unwrap().remove_file(file)?;
        if self.ref_count(file) == 0 {
            std::fs::remove_file(self.compose_path(file))?;
        }
        Ok(())
    }

    fn readdir(&self, dir: Inode) -> VaultResult<Vec<DirEntry>> {
        info!("readdir(dir={})", dir);
        self.database.lock().unwrap().readdir(dir)
    }
}
