use crate::database::Database;
use crate::local_vault::{LocalVault, RefCounter};
use crate::types::*;
use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
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
    local: Arc<LocalVault>,
    remote: Arc<Box<dyn Vault + Send + Sync>>,
    graveyard: PathBuf,
    // background: Sender<RemoteOp>,
    ref_count: RefCounter,
}

impl CachingVault {
    fn new(
        remote_vault: Box<dyn Vault + Send + Sync>,
        store_path: &Path,
    ) -> VaultResult<CachingVault> {
        let name = remote_vault.name();
        let graveyard = store_path.join("graveyard");
        if !graveyard.exists() {
            std::fs::create_dir(&graveyard)?
        }
        let remote = Arc::new(remote_vault);
        let local = Arc::new(LocalVault::new(&name, store_path)?);

        // let (send, recv): (Sender<RemoteOp>, Receiver<RemoteOp>) = mpsc::channel();
        // let background_remote = Arc::clone(&remote);
        // thread::spawn(move || run_background(background_remote));

        Ok(CachingVault {
            name,
            local: Arc::clone(&local),
            remote: Arc::clone(&remote),
            graveyard,
            // background: send,
            ref_count: RefCounter::new(),
        })
    }
}

fn network_err_to_option<T>(result: VaultResult<T>) -> VaultResult<Option<T>> {
    match result {
        Ok(val) => Ok(Some(val)),
        Err(VaultError::NetworkError(_)) => Ok(None),
        Err(err) => Err(err),
    }
}

fn run_background(remote: Arc<Box<dyn Vault + Send + Sync>>) {}

impl Vault for CachingVault {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn attr(&self, file: Inode) -> VaultResult<FileInfo> {
        self.remote.attr(file)
    }

    fn read(&self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>> {
        // Data is guaranteed to exist locally, because we fetch on open.
        self.local.read(file, offset, size)
    }

    fn write(&self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32> {
        self.local.write(file, offset, data)
    }

    fn open(&self, file: Inode, mode: OpenMode) -> VaultResult<()> {
        // Invariant: if ref_count > 0, then has local copy.
        if self.ref_count.count(file) > 0 {
            // Already opened.
            self.local.open(file, mode)?; // -> Will not throw NotExist.
        } else {
            // Not already opened. But at this point the file meta
            // must already exists on the local vault. Because when
            // userspace listed the parent directory, we add the
            // listed file to local vault (but don't fetch file data).
            // Now, the data is either not fetched (version = 0), or
            // out-of-date (version too low), or up-to-date.
            let our_version = self.local.attr(file)?.version;
            // FIXME: handle disconnect.
            let remote_meta = self.remote.attr(file)?;
            if our_version >= remote_meta.version {
                // Safe, keep using local version.
                ()
            } else {
                // Need to fetch from remote. TODO: read one chunk
                // at a time.
                let data = self.remote.read(file, 0, remote_meta.size as u32)?;
                self.local.write(file, 0, &data)?;
            }
        }
        self.ref_count.incf(file)?;
        Ok(())
    }

    fn close(&self, file: Inode) -> VaultResult<()> {
        // Is this the last close?
        if self.ref_count.count(file) == 1 {
            // Yes, perform close. FIXME: do things in background.
            // Duplicate the file to graveyard for 1) sole access 2)
            // it's going to the graveyard if write conflict occurs
            // anyway.
            let file_info = self.local.attr(file)?;
            let graveyard_file_path = self
                .graveyard
                .join(format!("{}-{}", file, file_info.version));
            self.local.copy_file(file, &graveyard_file_path)?;
            upload_file(file, &graveyard_file_path, Arc::clone(&self.remote))?;
        } else {
            // Not the last close, do nothing.
            ()
        }
        self.ref_count.decf(file)?;
        Ok(())
    }

    fn create(&self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        // FIXME: Handle disconnect.
        let inode = self.remote.create(parent, name, kind)?;
        // Readdir will fetch meta for the new file.
        self.readdir(parent)?;
        Ok(inode)
    }

    fn delete(&self, file: Inode) -> VaultResult<()> {
        // FIXME: Handle disconnect.
        if self.ref_count.count(file) == 0 {
            self.remote.delete(file)?;
            self.local.delete(file)
        } else {
            Ok(())
        }
    }

    fn readdir(&self, dir: Inode) -> VaultResult<Vec<FileInfo>> {
        // FIXME: Handle disconnection.
        for info in self.remote.readdir(dir)? {
            if !self.local.cache_has_file(info.inode)? {
                // Set version to 0 so file is fetched on open.
                self.local.cache_add_file(
                    dir, info.inode, &info.name, info.kind, info.atime, info.mtime, 0,
                )?;
            }
        }
        self.local.readdir(dir)
    }

    fn setup(&self) -> VaultResult<()> {
        // TODO: remote setup?
        self.local.setup()
    }

    fn tear_down(&self) -> VaultResult<()> {
        // TODO: remote tear_down?
        self.local.tear_down()
    }
}

fn upload_file(
    file: Inode,
    path: &Path,
    dest: Arc<Box<dyn Vault + Send + Sync>>,
) -> VaultResult<u32> {
    // FIXME: Add impl.
    Ok(0)
}
