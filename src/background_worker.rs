use crate::local_vault::LocalVault;
use crate::types::*;
use log::{error, info};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time;

pub type BackgroundLog = Arc<Mutex<Vec<BackgroundOp>>>;

pub struct BackgroundWorker {
    local: Arc<Mutex<LocalVault>>,
    remote: VaultRef,
    log: BackgroundLog,
    pending_log: Vec<BackgroundOp>,
    graveyard: PathBuf,
}

#[derive(Debug, Clone)]
pub enum BackgroundOp {
    Delete(Inode),
    Create(Inode, String, VaultFileType),
    Upload(Inode, String),
}

impl BackgroundWorker {
    /// Return a new background worker that fetches operations from
    /// log and performs them. Make sure to use _different_ `remote`
    /// for the background worker and the remote vault used by FUSE!
    /// This way background operation (like uploading large files)
    /// don't block FUSE operations.
    pub fn new(
        local: Arc<Mutex<LocalVault>>,
        remote: VaultRef,
        log: BackgroundLog,
        graveyard: &Path,
    ) -> BackgroundWorker {
        BackgroundWorker {
            local,
            remote,
            log,
            pending_log: vec![],
            graveyard: graveyard.to_path_buf(),
        }
    }

    /// Run the background worker, this never returns.
    pub fn run(&mut self) {
        // In each iteration, we collect new operations, append them
        // to the log, remove unnecessary ones, and try to perform
        // each one-by-one. If network error occurs, we save the
        // unfinished ones, and sleep for the next iteration.
        loop {
            thread::sleep(time::Duration::new(3, 0));
            // We resume from sleep,
            let mut new_log = {
                let mut shared_log = self.log.lock().unwrap();
                let log_copy = shared_log.clone();
                *shared_log = vec![];
                log_copy
            };
            // Collect new logs.
            self.pending_log.append(&mut new_log);
            // Remove unnecessary operations.
            let log = coalesce_ops(&self.pending_log);
            self.pending_log = vec![];

            // Perform each ops.
            let mut idx = 0;
            'sleep: while idx < log.len() {
                // Perform the operation
                let res = match log[idx] {
                    BackgroundOp::Delete(file) => self.handle_delete(file),
                    BackgroundOp::Create(parent, ref name, kind) => {
                        self.handle_create(parent, name, kind)
                    }
                    BackgroundOp::Upload(file, ref name) => self.handle_upload(file, name),
                };
                // If operation success or fail, move to next, if
                // connection broke, wait for a while and try again.
                match res {
                    Ok(_) => {
                        idx += 1;
                    }
                    Err(VaultError::RpcError(_)) => {
                        info!(
                            "Vault {} disconnected, retry in a sec",
                            self.remote.lock().unwrap().name()
                        );
                        // Add the unfinished ops to pending log, so
                        // next time when we wake up we continue from
                        // here.
                        self.pending_log = log[idx..].to_vec();

                        break 'sleep;
                    }
                    Err(err) => {
                        error!(
                            "Operation on vault {} failed: {:?} ",
                            self.remote.lock().unwrap().name(),
                            err
                        );
                        idx += 1
                    }
                };
            }
        }
    }

    fn handle_delete(&mut self, file: Inode) -> VaultResult<()> {
        info!("handle_delete({})", file);
        self.remote.lock().unwrap().delete(file)
    }

    fn handle_create(&mut self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<()> {
        info!(
            "handle_create(parent={}, name={}, kind={:?})",
            parent, name, kind
        );
        self.remote.lock().unwrap().create(parent, name, kind)?;
        Ok(())
    }

    fn handle_upload(&mut self, file: Inode, name: &str) -> VaultResult<()> {
        let vault_name = self.remote.lock().unwrap().name();
        let graveyard_file_path = self.graveyard.join(format!(
            "vault({})name({})inode({})",
            vault_name, name, file
        ));
        self.local
            .lock()
            .unwrap()
            .cache_copy_file(file, &graveyard_file_path)?;
        // FIXME: read by chunk.
        let mut buf = vec![];
        let mut fd = File::open(graveyard_file_path)?;
        fd.read_to_end(&mut buf)?;
        self.remote.lock().unwrap().write(file, 0, &buf)?;
        Ok(())
    }
}

/// Remote unnecessary operations in `ops`. For example, the write in
/// [write(A), delete(A)] can be removed.
fn coalesce_ops(ops: &[BackgroundOp]) -> Vec<BackgroundOp> {
    // TODO
    ops.to_vec()
}
