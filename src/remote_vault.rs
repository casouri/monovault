// Basically a gRPC client that makes requests to remote vault servers.

use crate::database::Database;
use crate::types::*;
use fuse::FileType;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug)]
pub struct RemoteVault {
    database: Arc<Database>,
}

// impl Vault for RemoteVault {
//
// }
