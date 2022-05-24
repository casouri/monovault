// Implement the FUSE API.

use crate::types::*;
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, ReplyWrite, Request,
};
use log::{debug, error, info, warn};
use std::boxed::Box;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::Arc;
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
    /// The order of the vaults in this vector cannot change for the
    /// duration of running.
    vaults: Vec<Arc<Box<dyn Vault>>>,
    /// Maps inode to its belonging vault.
    vault_map: HashMap<u64, Arc<Box<dyn Vault>>>,
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
        perm: 0o777,
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
        VaultError::NetworkError(_) => libc::EREMOTE,
        _ => libc::EIO,
    }
}

impl FS {
    pub fn new(vaults: Vec<Box<dyn Vault>>) -> FS {
        let mut vault_map = HashMap::new();
        let mut vault_base_map = HashMap::new();
        let mut vault_refs = vec![];
        let mut base = 1;
        for vault in vaults {
            let vault_ref = Arc::new(vault);
            let vault_base = base * (2 as u64).pow(48);
            vault_base_map.insert(vault_ref.name(), vault_base);
            vault_map.insert(1 + vault_base, Arc::clone(&vault_ref));
            vault_refs.push(vault_ref);
            base += 1;
        }
        FS {
            vaults: vault_refs,
            vault_map,
            vault_base_map,
        }
    }

    fn to_inner(&self, vault: &Box<dyn Vault>, file: Inode) -> Inode {
        file - self.vault_base_map.get(&vault.name()).unwrap()
    }

    fn to_outer(&self, vault: &Box<dyn Vault>, file: Inode) -> Inode {
        file + self.vault_base_map.get(&vault.name()).unwrap()
    }

    fn readdir_vaults(&self) -> Vec<(Inode, String, FileType)> {
        let mut result = vec![];
        result.push((1, ".".to_string(), FileType::Directory));
        result.push((1, "..".to_string(), FileType::Directory));
        for vault in &self.vaults {
            let root_inode = self.to_outer(vault, 1);
            result.push((root_inode, vault.name(), FileType::Directory));
        }
        debug!("readdir_vaults: {:?}", &result);
        result
    }

    fn get_vault(&self, inode: u64) -> VaultResult<Arc<Box<dyn Vault>>> {
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
                version: 0,                     // -> TODO: track this
            })
        } else {
            let vault = self.get_vault(_ino)?;
            let info = vault.attr(self.to_inner(&vault, _ino))?;
            Ok(FileInfo {
                // This is not used but we should do TRT.
                inode: self.to_outer(&vault, info.inode),
                name: info.name,       // This is not used
                kind: info.kind,       // This is used.
                size: info.size,       // This is used.
                atime: info.atime,     // This is used.
                mtime: info.mtime,     // This is used.
                version: info.version, // This is not used.
            })
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
        let vault = self.get_vault(parent)?;
        let inode = self.to_outer(
            &vault,
            vault.create(
                self.to_inner(&vault, parent),
                &name.to_string_lossy().into_owned(),
                VaultFileType::File,
            )?,
        );
        self.vault_map.insert(inode, Arc::clone(&vault));
        Ok(inode)
    }

    fn open_1(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32) -> VaultResult<()> {
        let vault = self.get_vault(_ino)?;
        // TODO: open mode.
        vault.open(self.to_inner(&vault, _ino), OpenMode::RW)
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
        let vault = self.get_vault(_ino)?;
        vault.close(self.to_inner(&vault, _ino))
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
        let vault = self.get_vault(ino)?;
        vault.read(self.to_inner(&vault, ino), offset, size)
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
        let vault = self.get_vault(ino)?;
        vault.write(self.to_inner(&vault, ino), offset, data)
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
                                let vault = self.get_vault(inode)?;
                                vault.delete(self.to_inner(&vault, inode))
                            }
                            (FileType::Directory, FileType::Directory) => {
                                // Actually do the work.
                                let vault = self.get_vault(inode)?;
                                vault.delete(self.to_inner(&vault, inode))
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
        let vault = self.get_vault(parent)?;
        let inode = vault.create(
            self.to_inner(&vault, parent),
            &name.to_string_lossy().into_owned(),
            VaultFileType::Directory,
        )?;
        let outer_inode = self.to_outer(&vault, inode);
        self.vault_map.insert(outer_inode, Arc::clone(&vault));
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
        let vault = self.get_vault(ino)?;
        let entries = vault.readdir(self.to_inner(&vault, ino))?;
        // Translate DirEntry to the tuple we return.
        let mut entries: Vec<(u64, String, FileType)> = entries
            .iter()
            .map(|entry| {
                // Remember the mapping from each entry to its vault.
                // When fuse starts up, it only has mappings for vault
                // roots, so any newly discovered files need to be
                // added to the map.
                let outer_inode = self.to_outer(&vault, entry.inode);
                if outer_inode != 1 {
                    self.vault_map.insert(outer_inode, Arc::clone(&vault));
                }
                (outer_inode, entry.name.clone(), translate_kind(entry.kind))
            })
            .collect();
        // If the directory is vault root, we need to add parent dir
        // for it.
        if self.to_inner(&vault, ino) == 1 {
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
        for vault in &self.vaults {
            match vault.tear_down() {
                Ok(_) => (),
                Err(err) => error!("destroy() => vault {} {:?}", vault.name(), err),
            }
        }
    }

    fn lookup(&mut self, _req: &Request, _parent: u64, _name: &std::ffi::OsStr, reply: ReplyEntry) {
        debug!(
            "lookup(parent={}, name={})",
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
                error!(
                    "lookup(parent={}, name={}) => {:?}",
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
                debug!(
                    "getattr({}) => (ino={}, kind={:?}, size={}, atime={}, mtime={})",
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
                error!("getattr({}) => {:?}", _ino, err);
                reply.error(translate_error(err))
            }
        }
    }

    fn setattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<time::SystemTime>,
        fh: Option<u64>,
        _crtime: Option<time::SystemTime>,
        _chgtime: Option<time::SystemTime>,
        _bkuptime: Option<time::SystemTime>,
        flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        info!(
            "setattr(ino={}, uid={:?}, gid={:?}, size={:?})",
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
                    "create(parent={}, name={}) => {}",
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
                    "create(parent={}, name={}) => {:?}",
                    parent,
                    name.to_string_lossy(),
                    err
                );
                reply.error(translate_error(err))
            }
        }
    }

    fn open(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        info!("open(ino={})", _ino);
        match self.open_1(_req, _ino, _flags) {
            Ok(_) => reply.opened(0, 0),
            Err(err) => {
                error!("open(ino={}) => {:?}", _ino, err);
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
        info!("release(ino={})", _ino);
        match self.release_1(_req, _ino, _fh, _flags, _lock_owner, _flush) {
            Ok(_) => reply.ok(),
            Err(err) => {
                error!("release(ino={}) => {:?}", _ino, err);
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
        info!("read(ino={}, offset={}, size={})", ino, offset, size);
        match self.read_1(_req, ino, fh, offset, size, flags, lock_owner) {
            Ok(data) => reply.data(&data),
            Err(err) => {
                error!(
                    "read(ino={}, offset={}, size={}) => {:?}",
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
        info!("write(ino={}, offset={})", ino, offset);
        match self.write_1(_req, ino, fh, offset, data, write_flags, flags, lock_owner) {
            Ok(size) => reply.written(size),
            Err(err) => {
                error!("write(ino={}, offset={}) =? {:?}", ino, offset, err);
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
        info!("flush(ino={})", ino);
        reply.ok();
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        info!("unlink(parent={}, name={})", parent, name.to_string_lossy());
        match self.unlink_1(_req, parent, name, FileType::RegularFile) {
            Ok(_) => reply.ok(),
            Err(err) => {
                error!(
                    "unlink(parent={}, name={}) => {:?}",
                    parent,
                    name.to_string_lossy(),
                    err
                );
                reply.error(translate_error(err))
            }
        }
    }

    fn opendir(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        info!("opendir(ino={})", _ino);
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
        info!("releasedir(ino={})", _ino);
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
        info!("mkdir(parent={}, name={})", parent, name.to_string_lossy());
        match self.mkdir_1(_req, parent, name, mode, umask) {
            Ok(inode) => {
                info!(
                    "mkdir(parent={}, name={}) => {}",
                    parent,
                    name.to_string_lossy(),
                    inode
                );
                // TODO: Use current time for atime and mtime.
                reply.entry(&ttl(), &attr(inode, FileType::Directory, 1, 0, 0), 0)
            }
            Err(err) => {
                error!(
                    "mkdir(parent={}, name={}) => {:?}",
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
        info!("readdir(ino={}, offset={})", ino, offset);
        match self.readdir_1(_req, ino, fh, offset) {
            Ok(inode_list) => {
                if (offset as usize) < inode_list.len() {
                    for idx in (offset as usize)..inode_list.len() {
                        let (inode, name, ty) = inode_list[idx].clone();
                        info!(
                            "reply.add(inode={}, offset={}, name={})",
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
                error!("readdir(ino={}, offset={}) => {:?}", ino, offset, err);
                reply.error(translate_error(err))
            }
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        info!("rmdir(parent={}, name={})", parent, name.to_string_lossy());
        if parent == 1 {
            // We don't allow deleting root and vault directories
            // (obviously). See rmdir(2) for detail on EBUSY.
            error!(
                "rmdir(parent={}, name={}) => EBUSY",
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
                    "rmdir(parent={}, name={}) => {:?}",
                    parent,
                    name.to_string_lossy(),
                    err
                );
                reply.error(translate_error(err))
            }
        }
    }
}
