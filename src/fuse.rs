// Implement the FUSE API.

use crate::types::*;
use fuse::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
};
use std::boxed::Box;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use time::Timespec;

// The main thing FS needs to do is to bend fuse api calls into vault
// api. For example, because we don't have permission, FS should just
// ignore any permission argument passed through FUSE api. It should
// also store a file descriptor to file path map, because FUSE expects
// to refer to files by file descriptors. It also needs to convert
// between types used by FUSE api and Vault api.
pub struct FS {
    config: Config,
    vaults: Vec<Arc<Box<dyn Vault>>>,
    vault_map: HashMap<u64, Arc<Box<dyn Vault>>>,
    next_inode: u64,
}

const error_code_placeholder: libc::c_int = 1;

fn ts() -> Timespec {
    Timespec::new(0, 0)
}

fn ttl() -> Timespec {
    Timespec::new(1, 0)
}

fn attr(kind: FileType) -> FileAttr {
    FileAttr {
        ino: 1,
        size: 0,
        blocks: 0,
        atime: ts(),
        mtime: ts(),
        ctime: ts(),
        crtime: ts(),
        kind,
        perm: 0o755,
        nlink: 0,
        uid: 0,
        gid: 0,
        rdev: 0,
        flags: 0,
    }
}

impl FS {
    pub fn new(config: Config, vaults: Vec<Box<dyn Vault>>) -> FS {
        let mut vault_map = HashMap::new();
        let mut vault_refs = vec![];
        let mut inode = 2;
        for vault in vaults {
            let vault_ref = Arc::new(vault);
            vault_map.insert(inode as u64, Arc::clone(&vault_ref));
            vault_refs.push(vault_ref);
            inode += 1;
        }
        FS {
            config,
            vaults: vault_refs,
            next_inode: 1,
            vault_map,
        }
    }

    fn readdir_vaults(&self) -> Vec<(Inode, String, FileType)> {
        let mut result = vec![];
        for (vault, inode) in self.vaults.iter().zip(2..self.vaults.len()) {
            result.push((inode as u64, vault.name(), FileType::Directory));
        }
        result
    }

    fn new_file(&mut self, path: &Path) -> u64 {
        self.next_inode += 1;
        self.next_inode
    }

    // fn get_vault<'a>(&self, path: &'a Path) -> VaultResult<(Box<dyn Vault>, &'a Path)> {
    //     let vec: Vec<&Path> = path.ancestors().collect();
    //     if vec.len() < 2 {
    //         return Err(VaultError::FileNotExist(
    //             path.to_string_lossy().into_owned(),
    //         ));
    //     }
    //     let vault = vec[vec.len() - 2];
    //     for vt in self.vaults {
    //         if vt.name() == vault.to_string_lossy().into_owned() {
    //             let rest = path.strip_prefix(vault).unwrap();
    //             return Ok((vt, rest));
    //         }
    //     }
    //     return Err(VaultError::FileNotExist(
    //         path.to_string_lossy().into_owned(),
    //     ));
    // }

    fn get_vault(&self, inode: u64) -> VaultResult<Arc<Box<dyn Vault>>> {
        if let Some(vault) = self.vault_map.get(&inode) {
            Ok(Arc::clone(vault))
        } else {
            Err(VaultError::FileNotExist(inode.to_string()))
        }
    }

    fn create_1(
        &mut self,
        _req: &fuse::Request,
        _parent: u64,
        _name: &std::ffi::OsStr,
        _mode: u32,
        _flags: u32,
    ) -> VaultResult<u64> {
        if _parent < 1024 {
            return Err(VaultError::InvalidAction(
                "You cannot create files here".to_string(),
            ));
        }
        let vault = self.get_vault(_parent)?;
        let inode = vault.create(
            _parent,
            &_name.to_string_lossy().into_owned(),
            VaultFileType::File,
        )?;
        self.vault_map.insert(inode, Arc::clone(&vault));
        Ok(inode)
    }

    fn open_1(&mut self, _req: &fuse::Request, _ino: u64, _flags: u32) -> VaultResult<()> {
        if _ino < 1024 {
            return Err(VaultError::InvalidAction(
                "You cannot open a vault as a file".to_string(),
            ));
        }
        let vault = self.get_vault(_ino)?;
        vault.open(_ino, &mut OpenOptions::new())
    }

    fn release_1(
        &mut self,
        _req: &fuse::Request,
        _ino: u64,
        _fh: u64,
        _flags: u32,
        _lock_owner: u64,
        _flush: bool,
    ) -> VaultResult<()> {
        if _ino < 1024 {
            return Err(VaultError::InvalidAction(
                "You cannot close a vault as a file".to_string(),
            ));
        }
        let vault = self.get_vault(_ino)?;
        vault.close(_ino)
    }

    fn read_1(
        &mut self,
        _req: &fuse::Request,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        _size: u32,
    ) -> VaultResult<Vec<u8>> {
        if _ino < 1024 {
            return Err(VaultError::InvalidAction(
                "You cannot read a vault as a file".to_string(),
            ));
        }
        let vault = self.get_vault(_ino)?;
        vault.read(_ino, _offset, _size)
    }

    fn write_1(
        &mut self,
        _req: &fuse::Request,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        _data: &[u8],
        _flags: u32,
    ) -> VaultResult<u32> {
        if _ino < 1024 {
            return Err(VaultError::InvalidAction(
                "You cannot write a vault as a file".to_string(),
            ));
        }
        let vault = self.get_vault(_ino)?;
        vault.write(_ino, _offset, _data)
    }

    fn unlink_1(&mut self, _req: &Request, _parent: u64, _name: &std::ffi::OsStr) {
        todo!()
    }

    fn mkdir_1(
        &mut self,
        _req: &fuse::Request,
        _parent: u64,
        _name: &std::ffi::OsStr,
        _mode: u32,
    ) -> VaultResult<u64> {
        let vault = self.get_vault(_parent)?;
        // If inode < 1024, it refers to a vault, ie, the root dir of
        // the vault, change inode to 1.
        let _parent = if _parent < 1024 { 1 } else { _parent };
        vault.create(
            _parent,
            &_name.to_string_lossy().into_owned(),
            VaultFileType::Directory,
        )
    }

    fn readdir_1(
        &mut self,
        _req: &fuse::Request,
        _ino: u64,
        _fh: u64,
        _offset: i64,
    ) -> VaultResult<Vec<(u64, String, FileType)>> {
        // If inode = 1, it refers to the root dir, list vaults.
        if _ino == 1 {
            return Ok(self.readdir_vaults());
        }
        let vault = self.get_vault(_ino)?;
        // If inode < 1024, it refers to a vault, ie, the root dir of
        // the vault, change inode to 1.
        let _ino = if _ino < 1024 { 1 } else { _ino };
        let entries = vault.readdir(_ino)?;
        Ok(entries
            .iter()
            .map(|entry| {
                (
                    entry.inode,
                    entry.name.clone(),
                    match entry.kind {
                        VaultFileType::File => FileType::RegularFile,
                        VaultFileType::Directory => FileType::Directory,
                    },
                )
            })
            .collect())
    }
}

impl Filesystem for FS {
    fn init(&mut self, _req: &fuse::Request) -> Result<(), libc::c_int> {
        Ok(())
    }

    fn destroy(&mut self, _req: &fuse::Request) {
        ()
    }

    fn create(
        &mut self,
        _req: &fuse::Request,
        _parent: u64,
        _name: &std::ffi::OsStr,
        _mode: u32,
        _flags: u32,
        reply: fuse::ReplyCreate,
    ) {
        match self.create_1(_req, _parent, _name, _mode, _flags) {
            Ok(inode) => reply.created(&ttl(), &attr(FileType::RegularFile), 0, 0, 0),
            Err(_) => reply.error(error_code_placeholder),
        }
    }

    fn open(&mut self, _req: &fuse::Request, _ino: u64, _flags: u32, reply: fuse::ReplyOpen) {
        match self.open_1(_req, _ino, _flags) {
            Ok(_) => reply.opened(0, 0),
            Err(_) => reply.error(error_code_placeholder),
        }
    }

    fn release(
        &mut self,
        _req: &fuse::Request,
        _ino: u64,
        _fh: u64,
        _flags: u32,
        _lock_owner: u64,
        _flush: bool,
        reply: fuse::ReplyEmpty,
    ) {
        match self.release_1(_req, _ino, _fh, _flags, _lock_owner, _flush) {
            Ok(_) => reply.ok(),
            Err(_) => reply.error(error_code_placeholder),
        }
    }

    fn read(
        &mut self,
        _req: &fuse::Request,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        _size: u32,
        reply: fuse::ReplyData,
    ) {
        match self.read_1(_req, _ino, _fh, _offset, _size) {
            Ok(data) => reply.data(&data),
            Err(_) => reply.error(error_code_placeholder),
        }
    }

    fn write(
        &mut self,
        _req: &fuse::Request,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        _data: &[u8],
        _flags: u32,
        reply: fuse::ReplyWrite,
    ) {
        match self.write_1(_req, _ino, _fh, _offset, _data, _flags) {
            Ok(size) => reply.written(size),
            Err(_) => reply.error(error_code_placeholder),
        }
    }

    fn flush(
        &mut self,
        _req: &fuse::Request,
        _ino: u64,
        _fh: u64,
        _lock_owner: u64,
        reply: fuse::ReplyEmpty,
    ) {
        reply.ok();
    }

    fn unlink(
        &mut self,
        _req: &Request,
        _parent: u64,
        _name: &std::ffi::OsStr,
        reply: fuse::ReplyEmpty,
    ) {
        todo!()
    }

    fn opendir(&mut self, _req: &fuse::Request, _ino: u64, _flags: u32, reply: fuse::ReplyOpen) {
        reply.opened(0, 0);
    }

    fn releasedir(
        &mut self,
        _req: &fuse::Request,
        _ino: u64,
        _fh: u64,
        _flags: u32,
        reply: fuse::ReplyEmpty,
    ) {
        reply.ok();
    }

    fn mkdir(
        &mut self,
        _req: &fuse::Request,
        _parent: u64,
        _name: &std::ffi::OsStr,
        _mode: u32,
        reply: fuse::ReplyEntry,
    ) {
        match self.mkdir_1(_req, _parent, _name, _mode) {
            Ok(size) => reply.entry(&ttl(), &attr(FileType::Directory), 0),
            Err(_) => reply.error(error_code_placeholder),
        }
    }

    fn readdir(
        &mut self,
        _req: &fuse::Request,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        mut reply: fuse::ReplyDirectory,
    ) {
        match self.readdir_1(_req, _ino, _fh, _offset) {
            Ok(inode_list) => {
                if (_offset as usize) < inode_list.len() {
                    for idx in (_offset as usize)..(inode_list.len() - 1) {
                        let (inode, name, ty) = inode_list[idx].clone();
                        // If return true, the reply buffer is full.
                        if reply.add(inode, idx as i64, ty, name) {
                            break;
                        }
                    }
                    reply.ok();
                }
            }
            Err(_) => reply.error(error_code_placeholder),
        }
    }

    fn rmdir(
        &mut self,
        _req: &fuse::Request,
        _parent: u64,
        _name: &std::ffi::OsStr,
        reply: fuse::ReplyEmpty,
    ) {
        todo!()
    }
}
