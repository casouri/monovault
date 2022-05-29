/// Implementation of Vault trait that actually stores files to disk.
use crate::database::Database;
use crate::types::*;
use log::{debug, info};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering::SeqCst},
    Arc, Mutex,
};
use std::time;

// TODO: modifying file currently doesn't update mtime and version of
// ancestor directories.

#[derive(Debug)]
pub struct RefCounter {
    ref_count: Mutex<HashMap<Inode, u64>>,
}

#[derive(Debug)]
pub struct FdMap {
    /// Name of this vault.
    name: String,
    /// Directory in which we store data files.
    data_file_dir: PathBuf,
    /// Maps inode to file handlers. DO NOT store references of fd's
    /// in other places, when the fd is removed from this map, we
    /// expect it to be dropped and the file closed.
    read_map: Mutex<HashMap<Inode, Arc<Mutex<File>>>>,
    write_map: Mutex<HashMap<Inode, Arc<Mutex<File>>>>,
}

/// Local vault delegates metadata work to the database, and mainly
/// works on locating the "data file" for each file, and reading and
/// writing data files.
#[derive(Debug)]
pub struct LocalVault {
    /// Name of this vault.
    name: String,
    /// Directory in which we store data files.
    data_file_dir: PathBuf,
    /// Database for metadata.
    database: Database,
    fd_map: FdMap,
    /// Counts the number of references to each file, when ref count
    /// of that file reaches 0, the file handler can be closed, and
    /// the file can be deleted from disk (if requested).
    ref_count: RefCounter,
    /// Records whether an opened file is modified (written).
    mod_track: RefCounter,
    /// The next allocated inode is current_inode + 1.
    current_inode: AtomicU64,
    /// Files waiting to be deleted.
    pending_delete: Vec<Inode>,
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
    pub fn zero(&self, file: Inode) {
        self.ref_count.lock().unwrap().remove(&file);
    }
}

impl FdMap {
    pub fn new(vault_name: &str, data_file_dir: &Path) -> FdMap {
        FdMap {
            name: vault_name.to_string(),
            data_file_dir: data_file_dir.to_path_buf(),
            read_map: Mutex::new(HashMap::new()),
            write_map: Mutex::new(HashMap::new()),
        }
    }

    /// Get the path to where the content of `file` is stored.
    /// Basically `db_path/vault_name-inode`.
    pub fn compose_path(&self, file: Inode, write: bool) -> PathBuf {
        self.data_file_dir.join(format!(
            "{}-{}{}",
            self.name,
            file.to_string(),
            if write { "-write" } else { "" }
        ))
    }

    /// Open and get the file handler for `file`. `file` is created if
    /// not already exists. When this function returns successfully,
    /// the data file must exist on disk (and `check_data_file_exists`
    /// returns true).
    pub fn get(&self, file: Inode, write: bool) -> VaultResult<Arc<Mutex<File>>> {
        let mut map = if write {
            self.write_map.lock().unwrap()
        } else {
            self.read_map.lock().unwrap()
        };
        match map.get(&file) {
            Some(fd) => Ok(Arc::clone(fd)),
            None => {
                let path = self.compose_path(file, write);
                info!("get_file, path={:?}", &path);
                // If create is true, either write or append must be
                // true.
                let mut fd = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .truncate(true)
                    .open(&path)?;
                // Make sure file is created.
                fd.flush()?;
                debug!("file content: {}", std::fs::read_to_string(&path)?);
                let fd_ref = Arc::new(Mutex::new(fd));
                map.insert(file, Arc::clone(&fd_ref));
                Ok(fd_ref)
            }
        }
    }

    pub fn take_over(&self, file: Inode) {
        let write_map = self.write_map.lock().unwrap();
        let write_fd = Arc::clone(&write_map.get(&file).unwrap());
        drop(write_map);
        self.read_map.lock().unwrap().insert(file, write_fd);
    }

    /// Drop `file` (and thus saving it to disk).
    pub fn close(&self, file: Inode, modified: bool) -> VaultResult<()> {
        self.read_map.lock().unwrap().remove(&file);
        self.write_map.lock().unwrap().remove(&file);

        if modified {
            std::fs::copy(
                self.compose_path(file, true),
                self.compose_path(file, false),
            )?;
            // If not modified, write is never called, a write copy is
            // never created, and we don't need to delete it.
            std::fs::remove_file(self.compose_path(file, true))?;
        }
        Ok(())
    }
}

/// The attr function used by both LocalVault and CachingRemote.
pub fn attr(file: Inode, database: &mut Database, fd_map: &FdMap) -> VaultResult<FileInfo> {
    // It is entirely valid (and possible) for the userspace to
    // refer to a file that doesn't exist in the database: when a
    // remote host deletes a file in our local vault, the
    // userspace on our host still remembers that file. If our
    // userspace now asks for that file, we can't throw a raw sql
    // error, we should throw a proper file not find.
    let mut info = match database.attr(file) {
        Ok(info) => Ok(info),
        Err(VaultError::SqliteError(rusqlite::Error::QueryReturnedNoRows)) => {
            Err(VaultError::FileNotExist(file))
        }
        Err(err) => Err(err),
    }?;
    let size = match info.kind {
        VaultFileType::File => {
            let meta = std::fs::metadata(fd_map.compose_path(file, false))?;
            meta.len()
        }
        VaultFileType::Directory => 1,
    };
    info.size = size;
    Ok(info)
}

/// The `read` function that is used by LocalVault and CachingRemote.
pub fn read(file: Inode, offset: i64, size: u32, fd_map: &FdMap) -> VaultResult<Vec<u8>> {
    let fd_lck = fd_map.get(file, false)?;
    let mut fd = fd_lck.lock().unwrap();
    let mut buf = vec![0; size as usize];
    if offset >= 0 {
        fd.seek(SeekFrom::Start(offset as u64))?;
    } else {
        fd.seek(SeekFrom::End(offset))?;
    }
    // Read exactly SIZE bytes, if not enough, read to EOF but don't
    // error.
    match fd.read_exact(&mut buf) {
        Ok(()) => Ok(buf),
        Err(err) => {
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                fd.read_to_end(&mut buf)?;
                Ok(buf)
            } else {
                Err(VaultError::IOError(err))
            }
        }
    }
}

pub fn write(file: Inode, offset: i64, data: &[u8], fd_map: &FdMap) -> VaultResult<u32> {
    let fd_lck = fd_map.get(file, true)?;
    let mut fd = fd_lck.lock().unwrap();

    if offset >= 0 {
        fd.seek(SeekFrom::Start(offset as u64))?;
    } else {
        fd.seek(SeekFrom::End(offset))?;
    }
    fd.write_all(data)?;
    // fd_map.take_over(file);
    Ok(data.len() as u32)
}

pub fn readdir(dir: Inode, database: &mut Database, fd_map: &FdMap) -> VaultResult<Vec<FileInfo>> {
    let (this, parent, entries) = database.readdir(dir)?;
    let mut result = vec![];
    for file in entries {
        result.push(attr(file, database, fd_map)?)
    }
    let mut current_dir = attr(this, database, fd_map)?;
    current_dir.name = ".".to_string();
    result.push(current_dir);
    if parent != 0 {
        let mut parrent_dir = attr(parent, database, fd_map)?;
        parrent_dir.name = "..".to_string();
        result.push(parrent_dir);
    }
    Ok(result)
}

/// Return true if the file meta exists in the vault.
pub fn has_file(file: Inode, database: &mut Database) -> VaultResult<bool> {
    // Invariant: metadata exists => data file exists.
    match database.attr(file) {
        Ok(_) => Ok(true),
        Err(VaultError::SqliteError(rusqlite::Error::QueryReturnedNoRows)) => Ok(false),
        Err(err) => Err(err),
    }
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
        let database = Database::new(&db_dir, name)?;
        let current_inode = { database.largest_inode() };
        info!("vault {} next_inode={}", name, current_inode);
        Ok(LocalVault {
            name: name.to_string(),
            database,
            fd_map: FdMap::new(name, &data_file_dir),
            data_file_dir,
            ref_count: RefCounter::new(),
            mod_track: RefCounter::new(),
            current_inode: AtomicU64::new(current_inode),
            pending_delete: vec![],
        })
    }

    /// Return a new inode.
    fn new_inode(&self) -> Inode {
        self.current_inode
            .fetch_update(SeqCst, SeqCst, |inode| Some(inode + 1))
            .unwrap();
        self.current_inode.load(SeqCst)
    }

    fn check_is_regular_file(&self, file: Inode) -> VaultResult<()> {
        let kind = self.database.attr(file)?.kind;
        match kind {
            VaultFileType::File => Ok(()),
            VaultFileType::Directory => Err(VaultError::IsDirectory(file)),
        }
    }

    /// Check if the corresponding data file for `file` exists on disk.
    fn check_data_file_exists(&self, file: Inode) -> VaultResult<()> {
        let path = self.fd_map.compose_path(file, false);
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

    fn tear_down(&mut self) -> VaultResult<()> {
        info!("tear_down()");
        let queue = &self.pending_delete;
        for &file in queue.iter() {
            std::fs::remove_file(self.fd_map.compose_path(file, false))?;
        }
        Ok(())
    }

    fn attr(&mut self, file: Inode) -> VaultResult<FileInfo> {
        debug!("attr({})", file);

        let info = attr(file, &mut self.database, &mut self.fd_map)?;

        debug!(
            "(inode={}, name={}, size={}, atime={}, mtime={}, kind={:?})",
            info.inode, info.name, info.size, info.atime, info.mtime, info.kind
        );
        Ok(info)
    }

    fn read(&mut self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>> {
        info!("read(file={}, offset={}, size={})", file, offset, size);
        // We don't access database during read because delete() will
        // remove the file from the database but before the last
        // close() is called, we still need to be able to serve read
        // and write.
        //
        // self.check_is_regular_file(file)?;
        self.check_data_file_exists(file)?;
        read(file, offset, size, &mut self.fd_map)
    }

    fn write(&mut self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32> {
        info!(
            "write(file={}, offset={}, size={})",
            file,
            offset,
            data.len()
        );
        // We don't access database during write because delete() will
        // remove the file from the database but before the last
        // close() is called, we still need to be able to serve read
        // and write.
        //
        // self.check_is_regular_file(file)?;
        self.check_data_file_exists(file)?;
        let size = write(file, offset, data, &mut self.fd_map)?;
        self.mod_track.incf(file)?;
        Ok(size as u32)
    }

    fn create(&mut self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        info!("create(parent={}, name={}, kind={:?})", parent, name, kind);
        let already_has_file = self.readdir(parent)?.iter().any(|info| info.name == name);
        if already_has_file {
            return Err(VaultError::FileAlreadyExist(parent, name.to_string()));
        }
        let inode = self.new_inode();
        // In fuse semantics (and thus vault's) create also open the
        // file. We need to call get_file to ensure the data file is
        // created.
        if let VaultFileType::File = kind {
            self.fd_map.get(inode, false)?;
        }
        // NOTE: Make sure we create data file before creating
        // metadata, to ensure consistency.
        let current_time = time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH)?
            .as_secs();
        self.database
            .add_file(parent, inode, name, kind, current_time, current_time, 1)?;
        self.ref_count.incf(inode)?;
        info!("created {}", inode);
        Ok(inode)
    }

    fn open(&mut self, file: Inode, mode: OpenMode) -> VaultResult<()> {
        info!(
            "open({}) ref_count {}->{}",
            file,
            self.ref_count.count(file),
            self.ref_count.count(file) + 1
        );
        self.check_is_regular_file(file)?;
        self.check_data_file_exists(file)?;
        self.ref_count.incf(file)?;
        Ok(())
    }

    fn close(&mut self, file: Inode) -> VaultResult<()> {
        // We don't access database during write because delete() will
        // remove the file from the database but before the last
        // close() is called, we still need to be able to serve read
        // and write.
        //
        // self.check_is_regular_file(file)?;
        self.check_data_file_exists(file)?;
        let count = self.ref_count.decf(file)?;
        info!(
            "close({}) ref_count {}->{}",
            file,
            // We don't want panic on under flow, so + rather than -.
            self.ref_count.count(file) + 1,
            self.ref_count.count(file)
        );
        if count == 0 {
            // Update mtime and version.
            let current_time = time::SystemTime::now()
                .duration_since(time::UNIX_EPOCH)?
                .as_secs();
            let modified = self.mod_track.nonzero(file);
            let version = self.database.attr(file)?.version;
            self.database.set_attr(
                file,
                None,
                Some(current_time),
                if modified { Some(current_time) } else { None },
                if modified { Some(version + 1) } else { None },
            )?;
            // When the file is dropped it is automatically closed. We
            // never store the file elsewhere and ref_count is 0 so
            // this is when the file is dropped.
            self.fd_map.close(file, modified)?;
            self.mod_track.zero(file);
        }
        Ok(())
    }

    fn delete(&mut self, file: Inode) -> VaultResult<()> {
        info!("delete({})", file);
        // Prefetch kind and store it, because we won't be able to
        // get it after deleting the file.
        let kind = self.database.attr(file)?.kind;
        // Database will check for nonempty directory for us.
        self.database.remove_file(file)?;
        // NOTE: Make sure we remove metadata before removing data
        // file, to ensure consistency.
        match kind {
            VaultFileType::File => {
                self.check_data_file_exists(file)?;
                if self.ref_count.count(file) == 0 {
                    std::fs::remove_file(self.fd_map.compose_path(file, false))?;
                } else {
                    // If there are other references to the file,
                    // don't delete yet.
                    let queue = &mut self.pending_delete;
                    if !queue.contains(&file) {
                        queue.push(file)
                    }
                }
            }
            VaultFileType::Directory => (),
        }
        Ok(())
    }

    fn readdir(&mut self, dir: Inode) -> VaultResult<Vec<FileInfo>> {
        debug!("readdir({})", dir);
        let result = readdir(dir, &mut self.database, &mut self.fd_map)?;
        debug!("readdir(dir={}) => {:?}", dir, &result);
        Ok(result)
    }
}

// Caching functions

// impl LocalVault {
//     /// Copy `file` to `path`.
//     pub fn cache_copy_file(&self, file: Inode, path: &Path) -> VaultResult<u64> {
//         let from_path = self.fd_map.compose_path(file);
//         let size = std::fs::copy(&from_path, path)?;
//         Ok(size)
//     }

// pub fn cache_set_attr(
//     &mut self,
//     file: Inode,
//     name: Option<&str>,
//     atime: Option<u64>,
//     mtime: Option<u64>,
//     version: Option<u64>,
// ) -> VaultResult<()> {
//     self.database.set_attr(file, name, atime, mtime, version)
// }

// Create metadata and data file for `child`. Set `version` to 0
// so content is fetched on open. This function should only be
// called when `child` does not exist. We have to have data file
// on disk because `attr` accesses it for size info.
// pub fn cache_add_file(
//     &mut self,
//     parent: Inode,
//     child: Inode,
//     name: &str,
//     kind: VaultFileType,
//     atime: u64,
//     mtime: u64,
//     version: u64,
// ) -> VaultResult<()> {
//     match kind {
//         VaultFileType::File => {
//             self.fd_map.get(child)?;
//         }
//         VaultFileType::Directory => (),
//     }
//     self.database
//         .add_file(parent, child, name, kind, atime, mtime, version)?;
//     Ok(())
// }

// pub fn cache_ref_count(&self, file: Inode) -> u64 {
//     self.ref_count.count(file)
// }
// }
