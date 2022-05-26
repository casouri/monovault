use crate::local_vault::{LocalVault, RefCounter};
use crate::types::*;
use log::{debug, info};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::time;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

// The caching vault first replicates data locally and send read/write
// request to remote vault in the background. Metadata requests are
// not cached locally and are directly handled by remote (for now), so
// there is no need for serialization between the background process
// and the foreground.

const CHECK_LIVENESS_INTERVAL: time::Duration = time::Duration::new(1, 0);

#[derive(Debug)]
struct RemoteOp {
    file: Inode,
    offset: i64,
    data: Vec<u8>,
}

pub struct CachingVault {
    name: String,
    // Just use another local vault to do the dirty work, I'm a
    // fucking genius.
    local: Arc<Mutex<LocalVault>>,
    remote: Arc<Mutex<Box<dyn Vault>>>,
    graveyard: PathBuf,
    // background: Sender<RemoteOp>,
}

impl CachingVault {
    pub fn new(
        remote_vault: Arc<Mutex<Box<dyn Vault>>>,
        store_path: &Path,
    ) -> VaultResult<CachingVault> {
        let name = remote_vault.lock().unwrap().name();
        let graveyard = store_path.join("graveyard");
        if !graveyard.exists() {
            std::fs::create_dir(&graveyard)?
        }
        let local = Arc::new(Mutex::new(LocalVault::new(&name, store_path)?));

        // let (send, recv): (Sender<RemoteOp>, Receiver<RemoteOp>) = mpsc::channel();
        // let background_remote = Arc::clone(&remote);
        // thread::spawn(move || run_background(background_remote));

        Ok(CachingVault {
            name,
            local: Arc::clone(&local),
            remote: Arc::clone(&remote_vault),
            graveyard,
            // background: send,
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

fn run_background(remote: Arc<Box<dyn Vault + Send + Sync>>) {}

impl Vault for CachingVault {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn attr(&mut self, file: Inode) -> VaultResult<FileInfo> {
        info!("attr({})", file);
        match self.remote.lock().unwrap().attr(file) {
            Ok(info) => Ok(info),
            Err(VaultError::FileNotExist(file)) => {
                self.local.lock().unwrap().delete(file)?;
                Err(VaultError::FileNotExist(file))
            }
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
            ()
        } else {
            // Not already opened. But at this point the file meta
            // must already exists on the local vault. Because when
            // userspace listed the parent directory, we add the
            // listed file to local vault (but don't fetch file data).
            // Now, the data is either not fetched (version = 0), or
            // out-of-date (version too low), or up-to-date.
            self.local.lock().unwrap().open(file, mode)?;
            let our_version = self.local.lock().unwrap().attr(file)?.version;
            // FIXME: handle disconnect.
            let remote_meta = self.remote.lock().unwrap().attr(file)?;
            if our_version >= remote_meta.version {
                // Safe, keep using local version.
                ()
            } else {
                // Need to fetch from remote. TODO: read one chunk
                // at a time.
                let data = self
                    .remote
                    .lock()
                    .unwrap()
                    .read(file, 0, remote_meta.size as u32)?;
                self.local.lock().unwrap().write(file, 0, &data)?;
            }
        }
        Ok(())
    }

    fn close(&mut self, file: Inode) -> VaultResult<()> {
        // We use open/close of local vault to track ref_count.
        self.local.lock().unwrap().close(file)?;
        let count = self.local.lock().unwrap().cache_ref_count(file);
        // We don't want panic on under flow, so + rather than -.
        info!("close({}) ref_count {}->{}", file, count + 1, count);
        // Is this the last close?
        if count == 0 {
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
            let graveyard_file_path = self.graveyard.join(format!(
                "vault({})name({})inode({})version({})",
                vault_name, info.name, file, info.version
            ));
            debug!("write: copy to {}", graveyard_file_path.to_string_lossy());
            self.local
                .lock()
                .unwrap()
                .copy_file(file, &graveyard_file_path)?;
            let remote = &mut self.remote.lock().unwrap();
            upload_file(file, &graveyard_file_path, remote)?;
        } else {
            // Not the last close, do nothing.
            ()
        }
        Ok(())
    }

    fn create(&mut self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        info!("create(parent={}, name={}, kind={:?})", parent, name, kind);
        // FIXME: Handle disconnect.
        let inode = self.remote.lock().unwrap().create(parent, name, kind)?;
        // Readdir will fetch meta for the new file.
        self.readdir(parent)?;
        Ok(inode)
    }

    fn delete(&mut self, file: Inode) -> VaultResult<()> {
        info!("delete({})", file);
        // FIXME: Handle disconnect. We don't wait for when ref_count
        // reaches 0. Remote and local vault will handle that.
        self.remote.lock().unwrap().delete(file)?;
        self.local.lock().unwrap().delete(file)
    }

    fn readdir(&mut self, dir: Inode) -> VaultResult<Vec<FileInfo>> {
        info!("readdir({})", dir);
        // FIXME: Handle disconnection.
        for info in self.remote.lock().unwrap().readdir(dir)? {
            // Obviously dir is already in the local vault, otherwise
            // userspace wouldn't call readdir on it. Now, for each of
            // its children, check if it exists in the cache and add
            // it if not.
            if !self.local.lock().unwrap().cache_has_file(info.inode)? {
                // Set version to 0 so file is fetched on open.
                self.local.lock().unwrap().cache_add_file(
                    dir, info.inode, &info.name, info.kind, info.atime, info.mtime, 0,
                )?;
            }
        }
        self.local.lock().unwrap().readdir(dir)
    }

    fn tear_down(&mut self) -> VaultResult<()> {
        // TODO: remote tear_down?
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
