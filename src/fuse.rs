// Implement the FUSE API.

use crate::local_vault::LocalVault;
use crate::remote_vault::RemoteVault;
use crate::types::*;
use fuse_mt::FilesystemMT;
use std::boxed::Box;
use std::path::Path;

// The main thing FS needs to do is to bend fuse api calls into vault
// api. For example, because we don't have permission, FS should just
// ignore any permission argument passed through FUSE api. It should
// also store a file descriptor to file path map, because FUSE expects
// to refer to files by file descriptors. It also needs to convert
// between types used by FUSE api and Vault api.
pub struct FS {
    config: Config,
    vaults: Vec<Box<dyn Vault>>,
}

impl FilesystemMT for FS {
    fn init(&self, _req: fuse_mt::RequestInfo) -> fuse_mt::ResultEmpty {
        todo!()
    }

    fn destroy(&self, _req: fuse_mt::RequestInfo) {
        todo!()
    }

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

    /// This function doesn't do anything because we don't flush until
    /// close. This is to keep the semantics simple when remote write
    /// comes into play.
    fn flush(
        &self,
        _req: fuse_mt::RequestInfo,
        _path: &Path,
        _fh: u64,
        _lock_owner: u64,
    ) -> fuse_mt::ResultEmpty {
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

    /// This function doesn't do anything because directories don't
    /// need to be opened or closed. But we need to add a entry to the
    /// file descriptor map and return that descriptor.
    fn opendir(
        &self,
        _req: fuse_mt::RequestInfo,
        _path: &Path,
        _flags: u32,
    ) -> fuse_mt::ResultOpen {
        todo!()
    }

    fn readdir(
        &self,
        _req: fuse_mt::RequestInfo,
        _path: &Path,
        _fh: u64,
    ) -> fuse_mt::ResultReaddir {
        todo!()
    }

    /// This function doesn't do anything because directories don't
    /// need to be opened or closed. But we should remove the file
    /// descriptor from our fild descriptor map.
    fn releasedir(
        &self,
        _req: fuse_mt::RequestInfo,
        _path: &Path,
        _fh: u64,
        _flags: u32,
    ) -> fuse_mt::ResultEmpty {
        todo!();
        fuse_mt::ResultEmpty::Ok(())
    }

    fn rmdir(
        &self,
        _req: fuse_mt::RequestInfo,
        _parent: &Path,
        _name: &std::ffi::OsStr,
    ) -> fuse_mt::ResultEmpty {
        todo!()
    }
}
