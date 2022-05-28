use crate::background_worker::{BackgroundLog, BackgroundOp, BackgroundWorker};
/// The caching vault first replicates data locally and send read/write
/// request to remote vault in the background.
use crate::local_vault::LocalVault;
use crate::types::*;
use log::{debug, info};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

pub struct CachingVault {
    /// Name of this vault, should be the same as the remote vault.
    name: String,
    // Just use another local vault to do the dirty work, I'm a
    // fucking genius.
    /// A local vault that is used to manage local copies.
    local: Arc<Mutex<LocalVault>>,
    /// The remote vault we are using.
    remote_map: HashMap<String, VaultRef>,
    log: BackgroundLog,
    /// Whether allow disconnected delete.
    allow_disconnected_delete: bool,
    /// Whether to allow disconnected create.
    allow_disconnected_create: bool,
}

impl CachingVault {
    /// The caching remote takes all the remotes rather than only the
    /// one it represents, because we want to be able savage from
    /// other vaults (asking B if it has a file of A cached).
    /// `remote_name` is the name of the vault this caching remote
    /// represents. `store_path` is the path to where we store
    /// database and data files. `remote_map` should contain all
    /// the remotes.
    pub fn new(
        remote_name: &str,
        remote_map: HashMap<String, VaultRef>,
        store_path: &Path,
        allow_disconnected_delete: bool,
        allow_disconnected_create: bool,
    ) -> VaultResult<CachingVault> {
        // Produce arguments for the background worker.
        let graveyard = store_path.join("graveyard");
        if !graveyard.exists() {
            std::fs::create_dir(&graveyard)?
        }
        let log = Arc::new(Mutex::new(vec![]));
        let local = Arc::new(Mutex::new(LocalVault::new(remote_name, store_path)?));
        let our_remote = remote_map
            .get(remote_name)
            .ok_or(VaultError::CannotFindVaultByName(remote_name.to_string()))?;
        let mut background_worker = BackgroundWorker::new(
            Arc::clone(&local),
            Arc::clone(our_remote),
            Arc::clone(&log),
            &graveyard,
        );
        let _handler = thread::spawn(move || background_worker.run());
        Ok(CachingVault {
            name: remote_name.to_string(),
            local,
            remote_map,
            log,
            allow_disconnected_delete,
            allow_disconnected_create,
        })
    }

    fn main(&self) -> VaultRef {
        Arc::clone(self.remote_map.get(&self.name).unwrap())
    }
}

fn rpc_err_to_option<T>(result: VaultResult<T>) -> VaultResult<Option<T>> {
    match result {
        Ok(val) => Ok(Some(val)),
        Err(VaultError::RpcError(_)) => Ok(None),
        Err(err) => Err(err),
    }
}

impl Vault for CachingVault {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn attr(&mut self, file: Inode) -> VaultResult<FileInfo> {
        info!("attr({})", file);
        match self.main().lock().unwrap().attr(file) {
            // Connected.
            Ok(info) => Ok(info),
            // Disconnected.
            Err(VaultError::RpcError(_)) => self.local.lock().unwrap().attr(file),
            // File is gone on remote.
            Err(VaultError::FileNotExist(file)) => {
                self.local.lock().unwrap().delete(file)?;
                Err(VaultError::FileNotExist(file))
            }
            // Other error.
            Err(err) => Err(err),
        }
    }

    fn read(&mut self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>> {
        info!("read(file={}, offset={}, size={})", file, offset, size);
        // Data is guaranteed to exist locally, because we fetch on open.
        self.local.lock().unwrap().read(file, offset, size)
    }

    fn write(&mut self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32> {
        info!(
            "write(file={}, offset={}, size={})",
            file,
            offset,
            data.len()
        );
        self.local.lock().unwrap().write(file, offset, data)
    }

    fn open(&mut self, file: Inode, mode: OpenMode) -> VaultResult<()> {
        let count = self.local.lock().unwrap().cache_ref_count(file);
        info!("open({}) ref_count {}->{}", file, count, count + 1);
        // We use open/close of local vault to track ref_count.
        self.local.lock().unwrap().open(file, mode)?;
        // Invariant: if ref_count > 0, then we have local copy.
        if count > 0 {
            // Already opened.
            return Ok(());
        }
        // Not already opened. But at this point the file meta
        // must already exists on the local vault. Because when
        // userspace listed the parent directory, we add the
        // listed file to local vault (but don't fetch file data).
        // Now, the data is either not fetched (version = 0), or
        // out-of-date (version too low), or up-to-date.
        let local = &mut self.local.lock().unwrap();
        match connected_case(local, self.main(), file) {
            Ok(()) => return Ok(()),
            Err(VaultError::RpcError(err)) => {
                return disconnected_case(local, file, VaultError::RpcError(err))
            }
            Err(err) => return Err(err),
        }
        // Download remote content if we are out-of-date.
        fn connected_case(
            local: &mut LocalVault,
            remote: VaultRef,
            file: Inode,
        ) -> VaultResult<()> {
            let mut remote = remote.lock().unwrap();
            let remote_meta = remote.attr(file)?;
            let our_version = local.attr(file)?.version;
            if our_version < remote_meta.version {
                // TODO: read by chunk.
                let data = remote.read(file, 0, remote_meta.size as u32)?;
                local.write(file, 0, &data)?;
            }
            Ok(())
        }
        // If remote is disconnected, use the local version if we have
        // one, report error if we don't.
        fn disconnected_case(
            local: &mut LocalVault,
            file: Inode,
            err: VaultError,
        ) -> VaultResult<()> {
            if local.attr(file)?.version != 0 {
                info!(
                    "open({}) => remote disconnected, we have a local copy",
                    file
                );
                Ok(())
            } else {
                info!(
                    "open({}) => remote disconnected, we don't have a local copy",
                    file
                );
                Err(err)
            }
        }
    }

    fn close(&mut self, file: Inode) -> VaultResult<()> {
        // We use open/close of local vault to track ref_count.
        self.local.lock().unwrap().close(file)?;
        let count = self.local.lock().unwrap().cache_ref_count(file);
        // We don't want panic on under flow, so use + rather than -.
        info!("close({}) ref_count {}->{}", file, count + 1, count);
        // Is this the last close?
        if count != 0 {
            return Ok(());
        }
        // Yes, perform close.
        let info = self.local.lock().unwrap().attr(file)?;
        debug!(
            "write: inode={}, name={}, size={}, atime={}, mtime={}, kind={:?}",
            info.inode, info.name, info.size, info.atime, info.mtime, info.kind
        );
        self.log
            .lock()
            .unwrap()
            .push(BackgroundOp::Upload(file, info.name));
        Ok(())
    }

    fn create(&mut self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        info!("create(parent={}, name={}, kind={:?})", parent, name, kind);
        let inode = match self.main().lock().unwrap().create(parent, name, kind) {
            // Connected.
            Ok(inode) => Ok(inode),
            // Disconnected.
            Err(VaultError::RpcError(_)) if self.allow_disconnected_create => {
                info!(
                    "create(parent={}, name={}, kind={:?}) => remote disconnect, creating locally",
                    parent, name, kind
                );
                self.log
                    .lock()
                    .unwrap()
                    .push(BackgroundOp::Create(parent, name.to_string(), kind));
                self.local.lock().unwrap().create(parent, name, kind)
            }
            // Other error.
            Err(err) => Err(err),
        }?;
        // Readdir will fetch meta for the new file.
        self.readdir(parent)?;
        Ok(inode)
    }

    fn delete(&mut self, file: Inode) -> VaultResult<()> {
        info!("delete({})", file);
        // We don't wait for when ref_count reaches 0. Remote and
        // local vault will handle that.
        match self.main().lock().unwrap().delete(file) {
            // Connected.
            Ok(_) => self.local.lock().unwrap().delete(file),
            // Disconnected.
            Err(VaultError::RpcError(_)) if self.allow_disconnected_delete => {
                info!("delete({}) => remote disconnected, deleting locally", file);
                self.log.lock().unwrap().push(BackgroundOp::Delete(file));
                self.local.lock().unwrap().delete(file)
            }
            // Other error.
            Err(err) => Err(err),
        }
    }

    fn readdir(&mut self, dir: Inode) -> VaultResult<Vec<FileInfo>> {
        info!("readdir({})", dir);
        match self.main().lock().unwrap().readdir(dir) {
            // Remote is accessible.
            Ok(entries) => {
                for info in entries {
                    // Obviously DIR is already in the local vault, otherwise
                    // userspace wouldn't call readdir on it. (Remote doesn't
                    // necessarily have it anymore, in that case we just
                    // return FNE.) Now, for each of its children, check if it
                    // exists in the cache and add it if not.
                    if !self.local.lock().unwrap().cache_has_file(info.inode)? {
                        // Set version to 0 so file is fetched on open.
                        self.local.lock().unwrap().cache_add_file(
                            dir, info.inode, &info.name, info.kind, info.atime, info.mtime, 0,
                        )?;
                    }
                }
                // Now we have everything in the local database, just
                // use that.
                self.local.lock().unwrap().readdir(dir)
            }
            // Disconnected.
            Err(VaultError::RpcError(err)) => {
                // Use local database if exists, otherwise return FNE.
                self.local.lock().unwrap().readdir(dir)
            }
            // Other error, report upward.
            Err(err) => Err(err),
        }
    }

    fn tear_down(&mut self) -> VaultResult<()> {
        // Remote doesn't need tear down.
        self.local.lock().unwrap().tear_down()
    }
}
