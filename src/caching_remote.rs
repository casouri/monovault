use crate::background_worker::{BackgroundLog, BackgroundOp, BackgroundWorker};
use crate::database::Database;
use crate::local_vault;
/// The caching vault first replicates data locally and send read/write
/// request to remote vault in the background.
use crate::local_vault::{FdMap, LocalVault, RefCounter};
use crate::types::*;
use log::{debug, info};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::{thread, time};

pub struct CachingVault {
    /// Name of this vault, should be the same as the remote vault.
    name: String,
    ref_count: RefCounter,
    mod_track: RefCounter,
    database: Database,
    fd_map: Arc<FdMap>,
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
        let our_remote = remote_map
            .get(remote_name)
            .ok_or(VaultError::CannotFindVaultByName(remote_name.to_string()))?;
        let data_file_dir = store_path.join("data");
        if !data_file_dir.exists() {
            std::fs::create_dir(&data_file_dir)?
        }
        let fd_map = Arc::new(FdMap::new(remote_name, &data_file_dir));
        let mut background_worker = BackgroundWorker::new(
            Arc::clone(&fd_map),
            Arc::clone(our_remote),
            Arc::clone(&log),
            &graveyard,
        );
        let _handler = thread::spawn(move || background_worker.run());
        // Create CachingVault.

        let db_dir = store_path.join("db");
        if !db_dir.exists() {
            std::fs::create_dir(&db_dir)?
        }
        Ok(CachingVault {
            name: remote_name.to_string(),
            ref_count: RefCounter::new(),
            mod_track: RefCounter::new(),
            fd_map,
            database: Database::new(&db_dir, remote_name)?,
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
        debug!("{}: attr({})", self.name(), file);
        match self.main().lock().unwrap().attr(file) {
            // Connected.
            Ok(info) => Ok(info),
            // Disconnected.
            Err(VaultError::RpcError(_)) => {
                local_vault::attr(file, &mut self.database, &mut self.fd_map)
            }
            // File is gone on remote.
            Err(VaultError::FileNotExist(file)) => {
                let kind = self.database.attr(file)?.kind;
                self.database.remove_file(file)?;
                // FIXME: delete_queue like local_vaule.
                if self.ref_count.count(file) == 0 {
                    std::fs::remove_file(self.fd_map.compose_path(file, false))?;
                }
                Err(VaultError::FileNotExist(file))
            }
            // Other error.
            Err(err) => Err(err),
        }
    }

    fn read(&mut self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>> {
        info!(
            "{}: read(file={}, offset={}, size={})",
            self.name(),
            file,
            offset,
            size
        );
        // Data is guaranteed to exist locally, because we fetch on open.
        local_vault::read(file, offset, size, &mut self.fd_map)
    }

    fn write(&mut self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32> {
        info!(
            "{}: write(file={}, offset={}, size={})",
            self.name(),
            file,
            offset,
            data.len()
        );
        let size = local_vault::write(file, offset, data, &mut self.fd_map)?;
        self.mod_track.incf(file)?;
        Ok(size)
    }

    fn open(&mut self, file: Inode, mode: OpenMode) -> VaultResult<()> {
        let count = self.ref_count.count(file);
        info!(
            "{}: open({}) ref_count {}->{}",
            self.name(),
            file,
            count,
            count + 1
        );
        // We use open/close of local vault to track ref_count.
        self.ref_count.incf(file)?;
        // Invariant: if ref_count > 0, then we have local copy.
        if count > 0 {
            // Already opened.
            return Ok(());
        }
        // Not already opened. But at this point the file meta must
        // already exists on the local vault. Because when userspace
        // listed the parent directory, we add the listed file to
        // local vault (but don't fetch file data). Now, the data is
        // either not fetched (version = 0), or out-of-date (version
        // too low), or up-to-date, or even more up-to-date, if we
        // have local changes not yet pushed to remote.
        match connected_case(self.main(), file, &mut self.database, &mut self.fd_map) {
            Ok(()) => return Ok(()),
            Err(VaultError::RpcError(err)) => {
                return disconnected_case(
                    file,
                    VaultError::RpcError(err),
                    &mut self.database,
                    &mut self.fd_map,
                )
            }
            Err(err) => return Err(err),
        }
        // Download remote content if we are out-of-date.
        fn connected_case(
            remote: VaultRef,
            file: Inode,
            database: &mut Database,
            fd_map: &FdMap,
        ) -> VaultResult<()> {
            let mut remote = remote.lock().unwrap();
            let remote_meta = remote.attr(file)?;
            let our_version = local_vault::attr(file, database, fd_map)?.version;
            debug!(
                "open({}) => local ver {}, remote ver {}",
                file, our_version, remote_meta.version
            );
            if our_version < remote_meta.version {
                // TODO: read by chunk. FIXME: Currently the data
                // could be newer than the version. Use download which
                // give us the version with the data.
                debug!("pulling from remote");
                let version = remote_meta.version;
                let data = remote.read(file, 0, remote_meta.size as u32)?;
                local_vault::write(file, 0, &data, fd_map)?;
                fd_map.close(file, true)?;
                database.set_attr(file, None, None, None, Some(version))?;
            }
            Ok(())
        }
        // If remote is disconnected, use the local version if we have
        // one, report error if we don't.
        fn disconnected_case(
            file: Inode,
            err: VaultError,
            database: &mut Database,
            fd_map: &FdMap,
        ) -> VaultResult<()> {
            if local_vault::attr(file, database, fd_map)?.version != 0 {
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
        self.ref_count.decf(file)?;
        let count = self.ref_count.count(file);
        // We don't want panic on under flow, so use + rather than -.
        info!(
            "{}: close({}) ref_count {}->{}",
            self.name(),
            file,
            count + 1,
            count
        );
        // Is this the last close?
        if count != 0 {
            return Ok(());
        }
        // Yes, perform close.
        let modified = self.mod_track.nonzero(file);
        if modified {
            self.mod_track.zero(file);
            let info = local_vault::attr(file, &mut self.database, &mut self.fd_map)?;
            debug!(
                "modified, write: inode={}, name={}, size={} (not accurate), atime={}, mtime={}, kind={:?}",
                file, info.name, info.size, info.atime, info.mtime, info.kind
            );
            // Increment the version so we don't fetch the remote
            // version upon next open.
            self.database
                .set_attr(file, None, None, None, Some(info.version + 1))?;
            self.fd_map.close(file, modified)?;
            // Add the op to background queue.
            self.log
                .lock()
                .unwrap()
                .push(BackgroundOp::Upload(file, info.name, info.version + 1));
        } else {
            self.fd_map.close(file, modified)?;
        }
        Ok(())
    }

    fn create(&mut self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        info!(
            "{}: create(parent={}, name={}, kind={:?})",
            self.name(),
            parent,
            name,
            kind
        );
        let inode = match self.main().lock().unwrap().create(parent, name, kind) {
            // Connected.
            Ok(inode) => {
                if let VaultFileType::File = kind {
                    self.fd_map.get(inode, false)?;
                }
                let current_time = time::SystemTime::now()
                    .duration_since(time::UNIX_EPOCH)?
                    .as_secs();
                self.database
                    .add_file(parent, inode, name, kind, current_time, current_time, 1)?;
                self.ref_count.incf(inode)?;
                Ok(inode)
            }
            // Disconnected.
            Err(VaultError::RpcError(_)) if self.allow_disconnected_create && false => {
                // FIXME: We don't allow disconnected create for now,
                // because that requires dealing with allocating
                // inodes.
                info!(
                    "create(parent={}, name={}, kind={:?}) => remote disconnect, creating locally",
                    parent, name, kind
                );
                Ok(0)
            }
            // Other error.
            Err(err) => Err(err),
        }?;
        // Readdir will fetch meta for the new file.
        self.readdir(parent)?;
        Ok(inode)
    }

    fn delete(&mut self, file: Inode) -> VaultResult<()> {
        info!("{}: delete({})", self.name(), file);
        // We don't wait for when ref_count reaches 0. Remote and
        // local vault will handle that.
        match self.main().lock().unwrap().delete(file) {
            // Connected.
            Ok(_) => {
                debug!("delete({}) => remote online", file);
                let kind = self.database.attr(file)?.kind;
                // FIXME: delete_queue and refactor.
                self.database.remove_file(file)?;
                if let VaultFileType::File = kind {
                    if self.ref_count.count(file) == 0 {
                        std::fs::remove_file(self.fd_map.compose_path(file, false))?;
                    }
                }
                Ok(())
            }
            // Disconnected.
            Err(VaultError::RpcError(_)) if self.allow_disconnected_delete => {
                info!("delete({}) => remote disconnected, deleting locally", file);
                self.log.lock().unwrap().push(BackgroundOp::Delete(file));
                // FIXME: delete_queue and refactor.
                let kind = self.database.attr(file)?.kind;
                self.database.remove_file(file)?;
                if let VaultFileType::File = kind {
                    if self.ref_count.count(file) == 0 {
                        std::fs::remove_file(self.fd_map.compose_path(file, false))?;
                    }
                }
                Ok(())
            }
            // Other error.
            Err(err) => Err(err),
        }
    }

    fn readdir(&mut self, dir: Inode) -> VaultResult<Vec<FileInfo>> {
        debug!("{}: readdir({})", self.name(), dir);
        match self.main().lock().unwrap().readdir(dir) {
            // Remote is accessible.
            Ok(entries) => {
                debug!("readdir({}) => remote online", dir);
                for info in entries {
                    // Obviously DIR is already in the local vault,
                    // otherwise userspace wouldn't call readdir on
                    // it. (Remote doesn't necessarily have it
                    // anymore, in that case we just return FNE.) Now,
                    // for each of its children, check if it exists in
                    // the cache and add it if not.
                    if !local_vault::has_file(info.inode, &mut self.database)? {
                        // Create an empty file.
                        if let VaultFileType::File = info.kind {
                            self.fd_map.get(info.inode, false)?;
                        }
                        // Set version to 0 so file is fetched on open.
                        self.database.add_file(
                            dir, info.inode, &info.name, info.kind, info.atime, info.mtime, 0,
                        )?;
                    }
                }
                // Now we have everything in the local database, just
                // use that.
                local_vault::readdir(dir, &mut self.database, &mut self.fd_map)
            }
            // Disconnected.
            Err(VaultError::RpcError(_)) => {
                debug!("readdir({}) => remote offline", dir);
                // Use local database if exists, otherwise return FNE.
                local_vault::readdir(dir, &mut self.database, &mut self.fd_map)
            }
            // Other error, report upward.
            Err(err) => Err(err),
        }
    }

    fn tear_down(&mut self) -> VaultResult<()> {
        // FIXME: delete_queue
        Ok(())
    }
}
