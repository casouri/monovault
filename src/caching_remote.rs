/// The caching vault first replicates data locally and send read/write
/// request to remote vault in the background.
use crate::local_vault::{LocalVault, RefCounter};
use crate::types::*;
use log::{debug, error, info};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time;

#[derive(Debug, Clone)]
pub enum BackgroundOp {
    Delete(Inode),
    Create(Inode, String, VaultFileType),
    Upload(Inode, PathBuf),
}

pub struct CachingVault {
    /// Name of this vault, should be the same as the remote vault.
    name: String,
    // Just use another local vault to do the dirty work, I'm a
    // fucking genius.
    /// A local vault that is used to manage local copies.
    local: Arc<Mutex<LocalVault>>,
    /// The remote vault we are using.
    remote: VaultRef,
    /// Path to the graveyard: where we store files that lost write
    /// conflict.
    graveyard: PathBuf,
    background_channel: Sender<BackgroundOp>,
    /// Whether allow disconnected delete.
    allow_disconnected_delete: bool,
    /// Whether to allow disconnected create.
    allow_disconnected_create: bool,
}

impl CachingVault {
    pub fn new(
        remote_vault: VaultRef,
        store_path: &Path,
        allow_disconnected_delete: bool,
        allow_disconnected_create: bool,
    ) -> VaultResult<CachingVault> {
        let name = remote_vault.lock().unwrap().name();
        let graveyard = store_path.join("graveyard");
        if !graveyard.exists() {
            std::fs::create_dir(&graveyard)?
        }
        let local = Arc::new(Mutex::new(LocalVault::new(&name, store_path)?));
        let (sender, recver) = channel();
        Ok(CachingVault {
            name,
            local: Arc::clone(&local),
            remote: Arc::clone(&remote_vault),
            graveyard,
            background_channel: sender,
            allow_disconnected_delete,
            allow_disconnected_create,
        })
    }
}

fn rpc_err_to_option<T>(result: VaultResult<T>) -> VaultResult<Option<T>> {
    match result {
        Ok(val) => Ok(Some(val)),
        Err(VaultError::RpcError(_)) => Ok(None),
        Err(err) => Err(err),
    }
}

fn run_background(remote: VaultRef, recver: Receiver<BackgroundOp>) {
    loop {
        match recver.recv() {
            Ok(op) => loop {
                match op {
                    BackgroundOp::Delete(file) => match remote.lock().unwrap().delete(file) {
                        Ok(_) => break,
                        Err(VaultError::RpcError(err)) => {
                            info!(
                                "Background delete({}) => cannot connect to remote vault, retry in a sec",
                                file
                            );
                            thread::sleep(time::Duration::new(3, 0));
                        }
                        Err(err) => {
                            error!("Background delete({}) => {:?}", file, err);
                        }
                    },
                    BackgroundOp::Create(parent, ref name, kind) => {
                        match remote.lock().unwrap().create(parent, &name, kind) {
                            Ok(_) => break,
                            Err(VaultError::RpcError(err)) => {
                                info!(
                                    "Background create(parent={}, child={}, kind={:?}) => cannot connect to remote vault, retry in a sec",
                                    parent, name, kind
                                );
                                thread::sleep(time::Duration::new(3, 0));
                            }
                            Err(err) => {
                                error!(
                                    "Background create(parent={}, child={}, kind={:?}) => {:?}",
                                    parent, name, kind, err
                                );
                            }
                        }
                    }
                    BackgroundOp::Upload(file, ref path) => (),
                }
            },
            Err(err) => {
                error!("Background helper channel closed, are we done?");
                return;
            }
        }
    }
}

impl Vault for CachingVault {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn attr(&mut self, file: Inode) -> VaultResult<FileInfo> {
        info!("attr({})", file);
        match self.remote.lock().unwrap().attr(file) {
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
        let remote = &mut self.remote.lock().unwrap();
        let local = &mut self.local.lock().unwrap();
        match connected_case(local, remote, file) {
            Ok(()) => return Ok(()),
            Err(VaultError::RpcError(err)) => {
                return disconnected_case(local, file, VaultError::RpcError(err))
            }
            Err(err) => return Err(err),
        }
        // Download remote content if we are out-of-date.
        fn connected_case(
            local: &mut LocalVault,
            remote: &mut Box<dyn Vault>,
            file: Inode,
        ) -> VaultResult<()> {
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
        // Yes, perform close. FIXME: do things in background.
        // Duplicate the file to graveyard for 1) sole access 2)
        // it's going to the graveyard if write conflict occurs
        // anyway.
        let info = self.local.lock().unwrap().attr(file)?;
        debug!(
            "write: inode={}, name={}, size={}, atime={}, mtime={}, kind={:?}",
            info.inode, info.name, info.size, info.atime, info.mtime, info.kind
        );
        let vault_name = self.remote.lock().unwrap().name();
        // We don't include the version in the name, so consecutive
        // closes on the same file produce only one copy.
        let graveyard_file_path = self.graveyard.join(format!(
            "vault({})name({})inode({})",
            vault_name, info.name, file
        ));
        debug!("write: copy to {}", graveyard_file_path.to_string_lossy());
        self.local
            .lock()
            .unwrap()
            .cache_copy_file(file, &graveyard_file_path)?;
        self.background_channel
            .send(BackgroundOp::Upload(file, graveyard_file_path))
            .unwrap();
        Ok(())
    }

    fn create(&mut self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        info!("create(parent={}, name={}, kind={:?})", parent, name, kind);
        let inode = match self.remote.lock().unwrap().create(parent, name, kind) {
            // Connected.
            Ok(inode) => Ok(inode),
            // Disconnected.
            Err(VaultError::RpcError(_)) if self.allow_disconnected_create => {
                info!(
                    "create(parent={}, name={}, kind={:?}) => remote disconnect, creating locally",
                    parent, name, kind
                );
                self.background_channel
                    .send(BackgroundOp::Create(parent, name.to_string(), kind))
                    .unwrap();
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
        match self.remote.lock().unwrap().delete(file) {
            // Connected.
            Ok(_) => self.local.lock().unwrap().delete(file),
            // Disconnected.
            Err(VaultError::RpcError(_)) if self.allow_disconnected_delete => {
                info!("delete({}) => remote disconnected, deleting locally", file);
                self.background_channel
                    .send(BackgroundOp::Delete(file))
                    .unwrap();
                self.local.lock().unwrap().delete(file)
            }
            // Other error.
            Err(err) => Err(err),
        }
    }

    fn readdir(&mut self, dir: Inode) -> VaultResult<Vec<FileInfo>> {
        info!("readdir({})", dir);
        match self.remote.lock().unwrap().readdir(dir) {
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

fn upload_file(file: Inode, path: &Path, dest: &mut Box<dyn Vault>) -> VaultResult<u32> {
    // FIXME: Transfer by chunks.
    let mut fd = File::open(path)?;
    let mut buf = vec![];
    fd.read_to_end(&mut buf)?;
    dest.write(file, 0, &buf)?;
    std::fs::remove_file(path)?;
    Ok(buf.len() as u32)
}
