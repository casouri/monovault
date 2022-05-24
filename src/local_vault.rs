// Implementation of Vault trait that actually stores files to disk.

use crate::database::Database;
use crate::types::*;
use log::{debug, info};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering::SeqCst},
    Arc, Mutex,
};
use std::time;

#[derive(Debug)]
pub struct RefCounter {
    ref_count: Mutex<HashMap<Inode, u64>>,
}

impl RefCounter {
    pub fn new() -> RefCounter {
        RefCounter {
            ref_count: Mutex::new(HashMap::new()),
        }
    }

    /// Increment ref count of `file`.
    pub fn incf(&self, file: Inode) -> VaultResult<u64> {
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
    pub fn decf(&self, file: Inode) -> VaultResult<u64> {
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
    pub fn count(&self, file: Inode) -> u64 {
        match self.ref_count.lock().unwrap().get(&file) {
            Some(&count) => count,
            None => 0,
        }
    }

    /// Return true if `file`'s count isn't zero.
    pub fn nonzero(&self, file: Inode) -> bool {
        match self.ref_count.lock().unwrap().get(&file) {
            Some(&count) => count != 0,
            None => false,
        }
    }

    /// Set `file`'s count to 0.
    pub fn set_to_zero(&self, file: Inode) {
        self.ref_count.lock().unwrap().remove(&file);
    }
}

/// Local vault delegates metadata work to the database, and mainly
/// works on locating the "data file" for each file, and reading and
/// writing data files.
#[derive(Debug)]
pub struct LocalVault {
    /// Name of this vault.
    name: String,
    data_file_dir: PathBuf,
    /// Database for metadata.
    database: Mutex<Database>,
    /// Maps inode to file handlers.
    fd_map: Mutex<HashMap<Inode, Arc<Mutex<File>>>>,
    /// Counts the number of references to each file, when ref count
    /// of that file reaches 0, the file handler can be closed, and
    /// the file can be deleted from disk (if requested).
    ref_count: RefCounter,
    /// Records whether an opened file is modified (written).
    mod_track: RefCounter,
    /// The next allocated inode is current_inode + 1.
    current_inode: AtomicU64,
    /// Files waiting to be deleted.
    pending_delete: Mutex<Vec<Inode>>,
}

impl LocalVault {
    /// `name` is the name of the vault, also the directory name of
    /// the vault root. `store_path` is the directory for database and
    /// data files. `store_path/db` contains databases and
    /// `store_path/data` contains data files.
    pub fn new(name: &str, store_path: &Path) -> VaultResult<LocalVault> {
        let data_file_dir = store_path.join("data");
        if !data_file_dir.exists() {
            std::fs::create_dir(&data_file_dir)?
        }
        let db_dir = store_path.join("db");
        if !db_dir.exists() {
            std::fs::create_dir(&db_dir)?
        }
        let mut database = Database::new(&db_dir, name)?;
        let current_inode = { database.largest_inode() };
        info!("vault {} next_inode={}", name, current_inode);
        Ok(LocalVault {
            name: name.to_string(),
            data_file_dir,
            database: Mutex::new(database),
            fd_map: Mutex::new(HashMap::new()),
            ref_count: RefCounter::new(),
            mod_track: RefCounter::new(),
            current_inode: AtomicU64::new(current_inode),
            pending_delete: Mutex::new(vec![]),
        })
    }

    /// Return a new inode.
    fn new_inode(&self) -> Inode {
        self.current_inode
            .fetch_update(SeqCst, SeqCst, |inode| Some(inode + 1))
            .unwrap();
        self.current_inode.load(SeqCst)
    }

    /// Get the path to where the content of `file` is stored.
    /// Basically `db_path/vault_name-inode`.
    fn compose_path(&self, file: Inode) -> PathBuf {
        self.data_file_dir
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
}

impl Vault for LocalVault {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn tear_down(&self) -> VaultResult<()> {
        info!("tear_down()");
        let queue = self.pending_delete.lock().unwrap();
        for &file in queue.iter() {
            std::fs::remove_file(self.compose_path(file))?;
        }
        Ok(())
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
        Ok(entry2info(&entry, size))
    }

    fn read(&self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>> {
        info!("read(file={}, offset={}, size={})", file, offset, size);
        // We don't access database during read because delete() will
        // remove the file from the database but before the last
        // close() is called, we still need to be able to serve read
        // and write.
        //
        // self.check_is_regular_file(file)?;
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
        // We don't access database during write because delete() will
        // remove the file from the database but before the last
        // close() is called, we still need to be able to serve read
        // and write.
        //
        // self.check_is_regular_file(file)?;
        self.check_data_file_exists(file)?;
        let lck = self.get_file(file)?;
        let mut fd = lck.lock().unwrap();
        let size = fd.write(data)?;
        self.mod_track.incf(file)?;
        Ok(size as u32)
    }

    fn create(&self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        info!("create(parent={}, name={}, kind={:?})", parent, name, kind);
        let inode = self.new_inode();
        // In fuse semantics (and thus vault's) create also open the
        // file. We need to call get_file to ensure the data file is
        // created.
        match kind {
            VaultFileType::File => {
                self.get_file(inode)?;
            }
            VaultFileType::Directory => (),
        }
        // NOTE: Make sure we create data file before creating
        // metadata, to ensure consistency.
        let current_time = time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH)?
            .as_secs();
        self.database
            .lock()
            .unwrap()
            .add_file(parent, inode, name, kind, current_time)?;
        self.ref_count.incf(inode)?;
        info!("created {}", inode);
        Ok(inode)
    }

    fn open(&self, file: Inode, mode: OpenMode) -> VaultResult<()> {
        info!("open(file={})", file);
        self.check_is_regular_file(file)?;
        self.check_data_file_exists(file)?;
        self.ref_count.incf(file)?;
        Ok(())
    }

    fn close(&self, file: Inode) -> VaultResult<()> {
        info!("close(file={})", file);
        // We don't access database during write because delete() will
        // remove the file from the database but before the last
        // close() is called, we still need to be able to serve read
        // and write.
        //
        // self.check_is_regular_file(file)?;
        self.check_data_file_exists(file)?;
        let count = self.ref_count.decf(file)?;
        if count == 0 {
            let mut map = self.fd_map.lock().unwrap();
            // When the file is dropped it is automatically closed. We
            // never store the file else where so this is when the
            // file is dropped (unless a concurrent read/write is
            // using it, but after the function ends it is dropped,
            // even that shouldn't be possible since ref count is 0
            // now).
            map.remove(&file);
            self.mod_track.set_to_zero(file);
        }
        let current_time = time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH)?
            .as_secs();
        let modified = self.mod_track.nonzero(file);
        // We hold the lock when retrieving version and setting it.
        let mut db_lock = self.database.lock().unwrap();
        let version = db_lock.attr(file)?.version;
        db_lock.set_attr(
            file,
            None,
            Some(current_time),
            if modified { Some(current_time) } else { None },
            if modified { Some(version + 1) } else { None },
        )?;
        Ok(())
    }

    fn delete(&self, file: Inode) -> VaultResult<()> {
        info!("delete(file={})", file);
        // Prefetch kind and store it, because we won't be able to
        // get it after deleting the file.
        let kind = self.database.lock().unwrap().attr(file)?.kind;
        // Database will check for nonempty directory for us.
        self.database.lock().unwrap().remove_file(file)?;
        // NOTE: Make sure we remove metadata before removing data
        // file, to ensure consistency.
        match kind {
            VaultFileType::File => {
                self.check_data_file_exists(file)?;
                if self.ref_count.count(file) == 0 {
                    std::fs::remove_file(self.compose_path(file))?;
                } else {
                    // If there are other references to the file,
                    // don't delete yet.
                    let mut queue = self.pending_delete.lock().unwrap();
                    if !queue.contains(&file) {
                        queue.push(file)
                    }
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
