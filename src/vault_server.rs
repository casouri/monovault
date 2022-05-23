// A gRPC server that receives requests and uses local_vault to do the
// actual work.

use crate::local_vault::LocalVault;
use crate::rpc;
use crate::rpc::vault_rpc_server::VaultRpc;
use crate::types::*;

struct VaultServer {
    local_vault: LocalVault,
}
