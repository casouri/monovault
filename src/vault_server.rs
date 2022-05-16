// A gRPC server that receives requests and uses local_vault to do the
// actual work.

use crate::local_vault::LocalVault;
use crate::types::*;

struct VaultServer {
    local_vault: LocalVault,
}
