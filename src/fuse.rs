// Implement the FUSE API.

use crate::types::*;
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, ReplyWrite, Request,
};
use libc::{EIO, ENOENT, ENOSYS};
use log::{debug, error, info};
use std::boxed::Box;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time;

// The main thing FS needs to do is to bend fuse api calls into vault
// api. For example, because we don't have permission, FS should just
// ignore any permission argument passed through FUSE api. It should
// also store a file descriptor to file path map, because FUSE expects
// to refer to files by file descriptors. It also needs to convert
// between types used by FUSE api and Vault api.
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

fn attr(ino: Inode, kind: FileType, size: u64) -> FileAttr {
    FileAttr {
        ino,
        size,
        blocks: 1,
        // Last access.
        atime: ts(),
        // Last modification.
        mtime: ts(),
        // Last change.
        ctime: ts(),
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
            })
        } else {
            let vault = self.get_vault(_ino)?;
            let info = vault.attr(self.to_inner(&vault, _ino))?;
            Ok(FileInfo {
                // This is not used but we should do TRT.
                inode: self.to_outer(&vault, info.inode),
                name: info.name, // This is not used
                kind: info.kind, // This is used.
                size: info.size, // This is used.
            })
        }
    }

    fn create_1(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
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
        vault.open(self.to_inner(&vault, _ino), &mut OpenOptions::new())
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
        fh: u64,
        offset: i64,
        size: u32,
        flags: i32,
        lock_owner: Option<u64>,
    ) -> VaultResult<Vec<u8>> {
        let vault = self.get_vault(ino)?;
        vault.read(self.to_inner(&vault, ino), offset, size)
    }

    fn write_1(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        write_flags: u32,
        flags: i32,
        lock_owner: Option<u64>,
    ) -> VaultResult<u32> {
        let vault = self.get_vault(ino)?;
        vault.write(self.to_inner(&vault, ino), offset, data)
    }

    fn unlink_1(&mut self, _req: &Request, _parent: u64, _name: &std::ffi::OsStr) {
        todo!()
    }

    fn mkdir_1(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
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
        fh: u64,
        offset: i64,
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
        Ok(())
    }

    fn destroy(&mut self) {
        ()
    }

    fn lookup(&mut self, _req: &Request, _parent: u64, _name: &std::ffi::OsStr, reply: ReplyEntry) {
        let name = _name.to_string_lossy().into_owned();
        debug!("lookup(parent={}, name={})", _parent, &name);
        match self.readdir_1(_req, _parent, 0, 0) {
            Ok(entries) => {
                // Find the child with NAME and return information of it.
                for (inode, fname, kind) in entries {
                    if fname == name {
                        debug!("lookup => (inode={}, kind={:?})", inode, kind);
                        // Lookup is only used for directories so the
                        // size can be just 1.
                        reply.entry(&ttl(), &attr(inode, kind, 1), 0);
                        return;
                    }
                }
                // No entry with the requested name, return error.
                reply.error(ENOENT);
            }
            Err(err) => {
                error!("{:?}", err);
                reply.error(translate_error(err));
            }
        }
    }

    fn getattr(&mut self, _req: &Request, _ino: u64, reply: ReplyAttr) {
        match self.getattr_1(_req, _ino) {
            Ok(entry) => {
                debug!(
                    "getattr({}) => (ino={}, kind={:?}, size={})",
                    _ino,
                    _ino,
                    translate_kind(entry.kind),
                    entry.size
                );
                reply.attr(&ttl(), &attr(_ino, translate_kind(entry.kind), entry.size))
            }
            Err(err) => {
                error!("{:?}", err);
                reply.error(translate_error(err))
            }
        }
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
                reply.created(&ttl(), &attr(inode, FileType::RegularFile, 0), 0, 0, 0)
            }
            Err(err) => {
                error!("{:?}", err);
                reply.error(translate_error(err))
            }
        }
    }

    fn open(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        info!("open(ino={})", _ino);
        match self.open_1(_req, _ino, _flags) {
            Ok(_) => reply.opened(0, 0),
            Err(err) => {
                error!("{:?}", err);
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
                error!("{:?}", err);
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
                error!("{:?}", err);
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
                error!("{:?}", err);
                reply.error(translate_error(err))
            }
        }
    }

    fn flush(&mut self, _req: &Request<'_>, ino: u64, fh: u64, lock_owner: u64, reply: ReplyEmpty) {
        info!("flush(ino={})", ino);
        reply.ok();
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        info!("unlink(parent={}, name={})", parent, name.to_string_lossy());
        todo!()
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
        match self.mkdir_1(_req, parent, name, mode, umask) {
            Ok(inode) => {
                info!(
                    "mkdir(parent={}, name={}) => {}",
                    parent,
                    name.to_string_lossy(),
                    inode
                );
                reply.entry(&ttl(), &attr(inode, FileType::Directory, 1), 0)
            }
            Err(err) => {
                error!("{:?}", err);
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
        match self.readdir_1(_req, ino, fh, offset) {
            Ok(inode_list) => {
                info!(
                    "readdir(ino={}, offset={}) => {:?}",
                    ino, offset, inode_list
                );
                if (offset as usize) < inode_list.len() {
                    for idx in (offset as usize)..inode_list.len() {
                        let (inode, name, ty) = inode_list[idx].clone();
                        debug!(
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
                    // reply.error(ENOENT);
                }
            }
            Err(err) => {
                error!("{:?}", err);
                reply.error(translate_error(err))
            }
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        info!("rmdir(parent={}, name={})", parent, name.to_string_lossy());
        todo!()
    }
}
