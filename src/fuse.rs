// Implement the FUSE API.

use crate::local_vault::LocalVault;
use crate::remote_vault::RemoteVault;
use crate::types::*;
use fuse_mt::FilesystemMT;
use std::boxed::Box;
use std::path::Path;

pub struct FS {
    config: Config,
    vaults: Vec<Box<dyn Vault>>,
}

impl FilesystemMT for FS {
    fn open(&self, _req: fuse_mt::RequestInfo, _path: &Path, _flags: u32) -> fuse_mt::ResultOpen {
        todo!()
    }

    fn read(
        &self,
        _req: fuse_mt::RequestInfo,
        _path: &Path,
        _fh: u64,
        _offset: u64,
        _size: u32,
        callback: impl FnOnce(fuse_mt::ResultSlice<'_>) -> fuse_mt::CallbackResult,
    ) -> fuse_mt::CallbackResult {
        todo!()
    }

    fn write(
        &self,
        _req: fuse_mt::RequestInfo,
        _path: &Path,
        _fh: u64,
        _offset: u64,
        _data: Vec<u8>,
        _flags: u32,
    ) -> fuse_mt::ResultWrite {
        todo!()
    }

    fn release(
        &self,
        _req: fuse_mt::RequestInfo,
        _path: &Path,
        _fh: u64,
        _flags: u32,
        _lock_owner: u64,
        _flush: bool,
    ) -> fuse_mt::ResultEmpty {
        todo!()
    }

    fn flush(
        &self,
        _req: fuse_mt::RequestInfo,
        _path: &Path,
        _fh: u64,
        _lock_owner: u64,
    ) -> fuse_mt::ResultEmpty {
        // We don't do anything for flush.
        fuse_mt::ResultEmpty::Ok(())
    }

    fn mkdir(
        &self,
        _req: fuse_mt::RequestInfo,
        _parent: &Path,
        _name: &std::ffi::OsStr,
        _mode: u32,
    ) -> fuse_mt::ResultEntry {
        todo!()
    }
}
