/// Implement the FUSE API.
use crate::types::*;
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, ReplyWrite, Request,
};
use log::{debug, error, info, log};
use std::boxed::Box;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time;

// The fuse layer does mainly two things: it translates between the
// global "outer" inodes and the vault-local "inner" inodes. And it
// remembers which file (inode) belongs to which vault and delegates
// requests to the correct vault.
//
// The mapping between global and local inode is necessary because
// each vault doesn't know or care about other vaults' inodes, they
// just start from 1 and go up. To avoid inode conflict between vaults
// when we put them all under a single file system, we chop u64 into a
// prefix and the actual inode. The first 16 bits are the prefix (so
// we support up to 2^16 vaults), and the last 48 bits are for inodes
// (so each vault can have up to 2^48 files). And for each inode in a
// vault, we translate it into the global inode by slapping the
// vault's prefix onto it.
pub struct FS {
    /// A vector of all the vaults, this is just for `readdir_vaults`.
    vaults: Vec<VaultRef>,
    /// Maps inode to its belonging vault.
    vault_map: HashMap<u64, VaultRef>,
    /// The base inode for each vault.
    vault_base_map: HashMap<String, u64>,
}

/// Return a dummy timestamp.
fn ts() -> time::SystemTime {
    time::SystemTime::UNIX_EPOCH
}

/// TTL tells how long the result should be kept in cache. Return a 30s TTL.
fn ttl() -> time::Duration {
    time::Duration::new(30, 0)
}

fn attr(ino: Inode, kind: FileType, size: u64, atime: u64, mtime: u64) -> FileAttr {
    FileAttr {
        ino,
        size,
        blocks: 1,
        // Last access.
        atime: time::UNIX_EPOCH
            .checked_add(time::Duration::new(atime, 0))
            .or(Some(ts()))
            .unwrap(),
        // Last modification.
        mtime: time::UNIX_EPOCH
            .checked_add(time::Duration::new(mtime, 0))
            .or(Some(ts()))
            .unwrap(),
        // Last change.
        ctime: time::UNIX_EPOCH
            .checked_add(time::Duration::new(mtime, 0))
            .or(Some(ts()))
            .unwrap(),
        // Creation time (macOS only).
        crtime: ts(),
        blksize: 1,
        kind,
        perm: match kind {
            FileType::RegularFile => 0o666,
            FileType::Directory => 0o777,
            _ => 0o666,
        },
        // Number of hard links.
        nlink: 1,
        uid: 1,
        gid: 1,
        // root device
        rdev: 0,
        /// Flags (macOS only, see chflags(2))
        flags: 0,
    }
}

fn translate_kind(kind: VaultFileType) -> FileType {
    match kind {
        VaultFileType::File => FileType::RegularFile,
        VaultFileType::Directory => FileType::Directory,
    }
}

fn translate_error(err: VaultError) -> libc::c_int {
    match err {
        VaultError::FileNameTooLong(_) => libc::ENAMETOOLONG,
        VaultError::NoCorrespondingVault(_) => libc::ENOENT,
        VaultError::FileNotExist(_) => libc::ENOENT,
        VaultError::NotDirectory(_) => libc::ENOTDIR,
        VaultError::IsDirectory(_) => libc::EISDIR,
        VaultError::DirectoryNotEmpty(_) => libc::ENOTEMPTY,
        VaultError::RemoteError(_) => libc::EREMOTE,
        VaultError::RpcError(_) => libc::ENETDOWN,
        _ => libc::EIO,
    }
}

/// Return true if `err` is truly common and generally can be ignored.
fn venial_error_p(err: &VaultError) -> bool {
    match err {
        // VaultError::FileNameTooLong(_) => true,
        VaultError::FileNotExist(_) => true,
        VaultError::FileAlreadyExist(_, _) => true,
        // VaultError::NotDirectory(_) => true,
        // VaultError::IsDirectory(_) => true,
        // VaultError::DirectoryNotEmpty(_) => true,
        _ => false,
    }
}

impl FS {
    pub fn new(vaults: Vec<VaultRef>) -> FS {
        let mut vault_map = HashMap::new();
        let mut vault_base_map = HashMap::new();
        let mut base = 1;
        for vault_lck in vaults.iter() {
            let vault_name = vault_lck.lock().unwrap().name();
            let vault_base = base * (2 as u64).pow(48);
            vault_base_map.insert(vault_name, vault_base);
            vault_map.insert(1 + vault_base, Arc::clone(&vault_lck));
            base += 1;
        }
        FS {
            vaults,
            vault_map,
            vault_base_map,
        }
    }

    fn to_inner(&self, vault_name: &str, file: Inode) -> Inode {
        file - self.vault_base_map.get(vault_name).unwrap()
    }

    fn to_outer(&self, vault_name: &str, file: Inode) -> Inode {
        file + self.vault_base_map.get(vault_name).unwrap()
    }

    fn readdir_vaults(&self) -> Vec<(Inode, String, FileType)> {
        let mut result = vec![];
        result.push((1, ".".to_string(), FileType::Directory));
        result.push((1, "..".to_string(), FileType::Directory));
        for vault_lck in &self.vaults {
            let vault = vault_lck.lock().unwrap();
            let root_inode = self.to_outer(&vault.name(), 1);
            result.push((root_inode, vault.name(), FileType::Directory));
        }
        debug!("readdir_vaults: {:?}", &result);
        result
    }

    fn get_vault(&self, inode: u64) -> VaultResult<VaultRef> {
        if let Some(vault) = self.vault_map.get(&inode) {
            Ok(Arc::clone(vault))
        } else {
            Err(VaultError::NoCorrespondingVault(inode))
        }
    }

    fn getattr_1(&mut self, _req: &Request, _ino: u64) -> VaultResult<FileInfo> {
        if _ino == 1 {
            Ok(FileInfo {
                inode: 1,                       // -> This is not used.
                name: "/".to_string(),          // -> This is not used.
                kind: VaultFileType::Directory, // -> This is used.
                size: 1,                        // -> This is used.
                atime: 0,                       // -> TODO: track this
                mtime: 0,                       // -> TODO: track this
                version: (1, 0),                // -> TODO: track this
            })
        } else {
            let vault_lck = self.get_vault(_ino)?;
            let mut vault = vault_lck.lock().unwrap();
            let vault_name = vault.name();
            let mut info = vault.attr(self.to_inner(&vault_name, _ino))?;
            info.inode = self.to_outer(&vault.name(), info.inode);
            Ok(info)
        }
    }

    fn lookup_1(
        &mut self,
        _req: &Request,
        _parent: u64,
        _name: &std::ffi::OsStr,
    ) -> VaultResult<FileInfo> {
        let name = _name.to_string_lossy().into_owned();
        let entries = self.readdir_1(_req, _parent, 0, 0)?;
        for (inode, fname, _) in entries {
            if fname == name {
                return self.getattr_1(_req, inode);
            }
        }
        Err(VaultError::FileNotExist(0))
    }

    fn create_1(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
    ) -> VaultResult<u64> {
        let vault_lck = self.get_vault(parent)?;
        let mut vault = vault_lck.lock().unwrap();
        let vault_name = vault.name();
        let inode = self.to_outer(
            &vault_name,
            vault.create(
                self.to_inner(&vault_name, parent),
                &name.to_string_lossy().into_owned(),
                VaultFileType::File,
            )?,
        );
        self.vault_map.insert(inode, Arc::clone(&vault_lck));
        Ok(inode)
    }

    fn open_1(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32) -> VaultResult<()> {
        let vault_lck = self.get_vault(_ino)?;
        let mut vault = vault_lck.lock().unwrap();
        let vault_name = vault.name();
        // TODO: open mode.
        vault.open(self.to_inner(&vault_name, _ino), OpenMode::RW)
    }

    fn release_1(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
    ) -> VaultResult<()> {
        let vault_lck = self.get_vault(_ino)?;
        let mut vault = vault_lck.lock().unwrap();
        let vault_name = vault.name();
        vault.close(self.to_inner(&vault_name, _ino))
    }

    fn read_1(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
    ) -> VaultResult<Vec<u8>> {
        let vault_lck = self.get_vault(ino)?;
        let mut vault = vault_lck.lock().unwrap();
        let vault_name = vault.name();
        vault.read(self.to_inner(&vault_name, ino), offset, size)
    }

    fn write_1(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
    ) -> VaultResult<u32> {
        let vault_lck = self.get_vault(ino)?;
        let mut vault = vault_lck.lock().unwrap();
        let vault_name = vault.name();
        vault.write(self.to_inner(&vault_name, ino), offset, data)
    }

    fn unlink_1(
        &mut self,
        _req: &Request,
        _parent: u64,
        _name: &std::ffi::OsStr,
        req_kind: FileType,
    ) -> VaultResult<()> {
        let name = _name.to_string_lossy().into_owned();
        match self.readdir_1(_req, _parent, 0, 0) {
            Ok(entries) => {
                // Find the child with NAME and return information of it.
                for (inode, fname, kind) in entries {
                    if fname == name {
                        return match (req_kind, kind) {
                            (FileType::RegularFile, FileType::Directory) => {
                                Err(VaultError::IsDirectory(inode))
                            }
                            (FileType::Directory, FileType::RegularFile) => {
                                Err(VaultError::NotDirectory(inode))
                            }
                            (FileType::RegularFile, FileType::RegularFile) => {
                                // Actually do the work.
                                let vault_lck = self.get_vault(inode)?;
                                let mut vault = vault_lck.lock().unwrap();
                                let vault_name = vault.name();
                                vault.delete(self.to_inner(&vault_name, inode))
                            }
                            (FileType::Directory, FileType::Directory) => {
                                // Actually do the work.
                                let vault_lck = self.get_vault(inode)?;
                                let mut vault = vault_lck.lock().unwrap();
                                let vault_name = vault.name();
                                vault.delete(self.to_inner(&vault_name, inode))
                            }
                            // Other types are impossible.
                            _ => Ok(()),
                        };
                    }
                }
                // No entry with the requested name, return error.
                return Err(VaultError::FileNotExist(0));
            }
            Err(err) => Err(err),
        }
    }

    fn mkdir_1(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
    ) -> VaultResult<Inode> {
        let vault_lck = self.get_vault(parent)?;
        let mut vault = vault_lck.lock().unwrap();
        let vault_name = vault.name();
        let inode = vault.create(
            self.to_inner(&vault_name, parent),
            &name.to_string_lossy().into_owned(),
            VaultFileType::Directory,
        )?;
        let outer_inode = self.to_outer(&vault.name(), inode);
        self.vault_map.insert(outer_inode, Arc::clone(&vault_lck));
        Ok(outer_inode)
    }

    fn readdir_1(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        _offset: i64,
    ) -> VaultResult<Vec<(u64, String, FileType)>> {
        // If inode = 1, it refers to the root dir, list vaults.
        if ino == 1 {
            return Ok(self.readdir_vaults());
        }
        let vault_lck = self.get_vault(ino)?;
        let mut vault = vault_lck.lock().unwrap();
        let name = vault.name();
        let entries = vault.readdir(self.to_inner(&name, ino))?;
        // Translate DirEntry to the tuple we return.
        let mut entries: Vec<(u64, String, FileType)> = entries
            .iter()
            .map(|entry| {
                // Remember the mapping from each entry to its vault.
                // When fuse starts up, it only has mappings for vault
                // roots, so any newly discovered files need to be
                // added to the map.
                let outer_inode = self.to_outer(&vault.name(), entry.inode);
                if outer_inode != 1 {
                    self.vault_map.insert(outer_inode, Arc::clone(&vault_lck));
                }
                (outer_inode, entry.name.clone(), translate_kind(entry.kind))
            })
            .collect();
        // If the directory is vault root, we need to add parent dir
        // for it.
        if self.to_inner(&vault.name(), ino) == 1 {
            entries.push((1, "..".to_string(), FileType::Directory))
        }
        Ok(entries)
    }
}

impl Filesystem for FS {
    fn init(
        &mut self,
        _req: &Request<'_>,
        _config: &mut fuser::KernelConfig,
    ) -> Result<(), libc::c_int> {
        info!("init()");
        Ok(())
    }

    fn destroy(&mut self) {
        info!("destroy()");
        for vault_lck in &self.vaults {
            match vault_lck.lock() {
                Ok(mut vault) => match vault.tear_down() {
                    Ok(_) => (),
                    Err(err) => error!("destroy() => vault {} {:?}", vault.name(), err),
                },
                Err(_) => (),
            }
        }
    }

    fn lookup(&mut self, _req: &Request, _parent: u64, _name: &std::ffi::OsStr, reply: ReplyEntry) {
        info!(
            "lookup(parent={:#x}, name={})",
            _parent,
            _name.to_string_lossy()
        );
        match self.lookup_1(_req, _parent, _name) {
            Ok(info) => reply.entry(
                &ttl(),
                &attr(
                    info.inode,
                    translate_kind(info.kind),
                    info.size,
                    info.atime,
                    info.mtime,
                ),
                0,
            ),
            Err(err) => {
                // NOTE: If you see lookup warning on werid stuff like
                // ._., ._xxx, etc, they are turd files (Apple double
                // files) like .DS_Store. See
                // https://code.google.com/archive/p/macfuse/wikis/OPTIONS.wiki.
                let level = if venial_error_p(&err) {
                    log::Level::Warn
                } else {
                    log::Level::Error
                };
                log!(
                    level,
                    "lookup(parent={:#x}, name={}) => {:?}",
                    _parent,
                    _name.to_string_lossy(),
                    err
                );
                reply.error(translate_error(err));
            }
        }
    }

    fn getattr(&mut self, _req: &Request, _ino: u64, reply: ReplyAttr) {
        match self.getattr_1(_req, _ino) {
            Ok(entry) => {
                info!(
                    "getattr({}) => (ino={:#x}, kind={:?}, size={}, atime={}, mtime={})",
                    _ino,
                    _ino,
                    translate_kind(entry.kind),
                    entry.size,
                    entry.atime,
                    entry.mtime,
                );
                reply.attr(
                    &ttl(),
                    &attr(
                        _ino,
                        translate_kind(entry.kind),
                        entry.size,
                        entry.atime,
                        entry.mtime,
                    ),
                )
            }
            Err(err) => {
                error!("getattr({:#x}) => {:?}", _ino, err);
                reply.error(translate_error(err))
            }
        }
    }

    fn setattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<time::SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<time::SystemTime>,
        _chgtime: Option<time::SystemTime>,
        _bkuptime: Option<time::SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        info!(
            "setattr(ino={:#x}, uid={:?}, gid={:?}, size={:?})",
            ino, uid, gid, size
        );
        self.getattr(_req, ino, reply)
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        match self.create_1(_req, parent, name, mode, umask, flags) {
            Ok(inode) => {
                info!(
                    "create(parent={:#x}, name={}) => {}",
                    parent,
                    name.to_string_lossy(),
                    inode
                );
                reply.created(
                    &ttl(),
                    // TODO: use current time for atime and mtime instead.
                    &attr(inode, FileType::RegularFile, 0, 0, 0),
                    0,
                    0,
                    0,
                )
            }
            Err(err) => {
                error!(
                    "create(parent={:#x}, name={}) => {:?}",
                    parent,
                    name.to_string_lossy(),
                    err
                );
                reply.error(translate_error(err))
            }
        }
    }

    fn open(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        info!("open({:#x})", _ino);
        match self.open_1(_req, _ino, _flags) {
            Ok(_) => reply.opened(0, 0),
            Err(err) => {
                error!("open({:#x}) => {:?}", _ino, err);
                reply.error(translate_error(err))
            }
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        info!("release({:#x})", _ino);
        match self.release_1(_req, _ino, _fh, _flags, _lock_owner, _flush) {
            Ok(_) => reply.ok(),
            Err(err) => {
                error!("release({:#x}) => {:?}", _ino, err);
                reply.error(translate_error(err))
            }
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        flags: i32,
        lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        info!("read(ino={:#x}, offset={}, size={})", ino, offset, size);
        match self.read_1(_req, ino, fh, offset, size, flags, lock_owner) {
            Ok(data) => reply.data(&data),
            Err(err) => {
                error!(
                    "read(ino={:#x}, offset={}, size={}) => {:?}",
                    ino, offset, size, err
                );
                reply.error(translate_error(err))
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        write_flags: u32,
        flags: i32,
        lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        info!(
            "write(ino={:#x}, offset={}, size={})",
            ino,
            offset,
            data.len()
        );
        match self.write_1(_req, ino, fh, offset, data, write_flags, flags, lock_owner) {
            Ok(size) => reply.written(size),
            Err(err) => {
                error!("write(ino={:#x}, offset={}) =? {:?}", ino, offset, err);
                reply.error(translate_error(err))
            }
        }
    }

    fn flush(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        _lock_owner: u64,
        reply: ReplyEmpty,
    ) {
        info!("flush({:#x})", ino);
        reply.ok();
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        info!(
            "unlink(parent={:#x}, name={})",
            parent,
            name.to_string_lossy()
        );
        match self.unlink_1(_req, parent, name, FileType::RegularFile) {
            Ok(_) => reply.ok(),
            Err(err) => {
                error!(
                    "unlink(parent={:#x}, name={}) => {:?}",
                    parent,
                    name.to_string_lossy(),
                    err
                );
                reply.error(translate_error(err))
            }
        }
    }

    fn opendir(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        info!("opendir({:#x})", _ino);
        reply.opened(0, 0);
    }

    fn releasedir(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _flags: i32,
        reply: ReplyEmpty,
    ) {
        info!("releasedir({:#x})", _ino);
        reply.ok();
    }

    fn mkdir(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        info!(
            "mkdir(parent={:#x}, name={})",
            parent,
            name.to_string_lossy()
        );
        match self.mkdir_1(_req, parent, name, mode, umask) {
            Ok(inode) => {
                info!(
                    "mkdir(parent={:#x}, name={}) => {}",
                    parent,
                    name.to_string_lossy(),
                    inode
                );
                // TODO: Use current time for atime and mtime.
                reply.entry(&ttl(), &attr(inode, FileType::Directory, 1, 0, 0), 0)
            }
            Err(err) => {
                let level = if venial_error_p(&err) {
                    log::Level::Warn
                } else {
                    log::Level::Error
                };
                log!(
                    level,
                    "mkdir(parent={:#x}, name={}) => {:?}",
                    parent,
                    name.to_string_lossy(),
                    err
                );
                reply.error(translate_error(err))
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        info!("readdir(ino={:#x}, offset={})", ino, offset);
        match self.readdir_1(_req, ino, fh, offset) {
            Ok(inode_list) => {
                if (offset as usize) < inode_list.len() {
                    for idx in (offset as usize)..inode_list.len() {
                        let (inode, name, ty) = inode_list[idx].clone();
                        info!(
                            "reply.add(inode={:#x}, offset={}, name={})",
                            inode,
                            idx + 1,
                            &name
                        );
                        // If return true, the reply buffer is full.
                        if reply.add(inode, idx as i64 + 1, ty, name) {
                            break;
                        }
                    }
                    // Added enough entries, return.
                    reply.ok();
                } else {
                    // Offset too large, no more entries.
                    debug!("readdir: return empty");
                    reply.ok();
                }
            }
            Err(err) => {
                error!("readdir(ino={:#x}, offset={}) => {:?}", ino, offset, err);
                reply.error(translate_error(err))
            }
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        info!(
            "rmdir(parent={:#x}, name={})",
            parent,
            name.to_string_lossy()
        );
        if parent == 1 {
            // We don't allow deleting root and vault directories
            // (obviously). See rmdir(2) for detail on EBUSY.
            error!(
                "rmdir(parent={:#x}, name={}) => EBUSY",
                parent,
                name.to_string_lossy()
            );
            reply.error(libc::EBUSY);
            return;
        }
        match self.unlink_1(_req, parent, name, FileType::Directory) {
            Ok(_) => reply.ok(),
            Err(err) => {
                error!(
                    "rmdir(parent={:#x}, name={}) => {:?}",
                    parent,
                    name.to_string_lossy(),
                    err
                );
                reply.error(translate_error(err))
            }
        }
    }
}
