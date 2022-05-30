use crate::rpc::vault_rpc_server::VaultRpc;
/// A gRPC server that receives requests and uses local_vault to do the
/// actual work.
use crate::rpc::{vault_rpc_server, Acceptance};
use crate::rpc::{
    DataChunk, DirEntryList, Empty, FileInfo, FileToCreate, FileToOpen, FileToRead, FileToWrite,
    Grail, Inode, Size,
};
use crate::types::{
    unpack_to_caching, CompressedError, OpenMode, Vault, VaultError, VaultFileType, VaultRef,
    VaultResult, GRPC_DATA_CHUNK_SIZE,
};
use async_trait::async_trait;
use log::{debug, info};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

pub fn run_server(
    address: &str,
    local_name: &str,
    vault_map: HashMap<String, VaultRef>,
    runtime: Arc<Runtime>,
) {
    let service = vault_rpc_server::VaultRpcServer::new(
        VaultServer::new(local_name, vault_map).expect("Cannot create server instance"),
    );
    let server = tonic::transport::Server::builder().add_service(service.clone());
    let incoming = match runtime.block_on(TcpListener::bind(address)) {
        Ok(lis) => tokio_stream::wrappers::TcpListenerStream::new(lis),
        Err(err) => panic!("Cannot listen to address: {:?}", err),
    };
    info!("Server started");
    runtime
        .block_on(server.serve_with_incoming(incoming))
        .expect("Error serving requests");
}

pub struct VaultServer {
    vault_map: HashMap<String, VaultRef>,
    local_name: String,
}

impl VaultServer {
    /// `vault_map` should contain all the remote and local vault.
    pub fn new(local_name: &str, vault_map: HashMap<String, VaultRef>) -> VaultResult<VaultServer> {
        if vault_map.get(local_name).is_none() {
            return Err(VaultError::CannotFindVaultByName(local_name.to_string()));
        }
        Ok(VaultServer {
            local_name: local_name.to_string(),
            vault_map,
        })
    }

    fn local(&self) -> &VaultRef {
        self.vault_map.get(&self.local_name).unwrap()
    }
}

/// Translate VaultFileType to rpc message field.
fn kind2num(v: VaultFileType) -> i32 {
    let k = match v {
        VaultFileType::File => 1,
        VaultFileType::Directory => 2,
    };
    return k;
}

/// Translate rpc message field to VaultFileType.
fn num2kind(k: i32) -> VaultFileType {
    if k == 1 {
        return VaultFileType::File;
    } else {
        return VaultFileType::Directory;
    }
}

/// Translate some of the errors to status code and others to a
/// catch-all status.
fn translate_result<T>(res: VaultResult<T>) -> Result<T, Status> {
    match res {
        Ok(val) => Ok(val),
        Err(err) => Err(pack_status(err)),
    }
}

fn pack_status(err: VaultError) -> Status {
    let compressed_err: CompressedError = err.into();
    let encoded = serde_json::to_string(&compressed_err).unwrap();
    Status::not_found(encoded)
}

#[async_trait]
impl VaultRpc for VaultServer {
    async fn attr(&self, request: Request<Inode>) -> Result<Response<FileInfo>, Status> {
        let inner = request.into_inner();
        info!("attr({})", inner.value);
        let res = translate_result(self.local().lock().unwrap().attr(inner.value))?;
        Ok(Response::new(FileInfo {
            inode: res.inode,
            name: res.name,
            kind: kind2num(res.kind),
            size: res.size,
            atime: res.atime,
            mtime: res.mtime,
            version: res.version,
        }))
    }
    type readStream = ReceiverStream<Result<DataChunk, Status>>;
    type savageStream = ReceiverStream<Result<DataChunk, Status>>;

    async fn read(
        &self,
        request: Request<FileToRead>,
    ) -> Result<Response<Self::readStream>, Status> {
        let request_inner = request.into_inner();
        info!(
            "read(file={}, offset={}, size={})",
            request_inner.file, request_inner.offset, request_inner.size
        );
        // Don't lock the vault when transferring data on wire. Get
        // data and version from local vault.
        let (data, version) = {
            let mut vault = self.local().lock().unwrap();
            let data = translate_result(vault.read(
                request_inner.file,
                request_inner.offset,
                request_inner.size,
            ))?;
            let version = translate_result(vault.attr(request_inner.file))?.version;
            (data, version)
        };
        // Create the stream that sends messages.
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let mut offset = request_inner.offset as usize;
            let blk_size = GRPC_DATA_CHUNK_SIZE;
            while offset < data.len() {
                let end = std::cmp::min(offset + blk_size, data.len());
                let reply = DataChunk {
                    payload: data[offset..end].to_vec(),
                    version,
                };
                tx.send(Ok(reply)).await.unwrap();
                offset = end;
            }
        });
        // Return the stream.
        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn savage(
        &self,
        request: Request<Grail>,
    ) -> Result<Response<Self::savageStream>, Status> {
        let req = request.into_inner();
        info!("savage(vault={}, file={})", req.vault, req.file);
        // Get data and version from the caching remote vault.
        let result: VaultResult<(Vec<u8>, u64)> = {
            match self.vault_map.get(&req.vault) {
                None => {
                    debug!("We don't know this vault");
                    Err(VaultError::FileNotExist(req.file))
                }
                Some(vault) => {
                    let mut vault = vault.lock().unwrap();
                    let caching_remote = translate_result(unpack_to_caching(&mut vault));
                    if let Err(_) = &caching_remote {
                        debug!("Cannot translate to caching remote, probably because we didn't enable caching");
                    }
                    let caching_remote = caching_remote?;
                    caching_remote.search_in_cache(req.file)
                }
            }
        };
        if let Err(VaultError::FileNotExist(_)) = result {
            debug!("We can't find the file in cache");
        }
        let (data, version) = translate_result(result)?;
        debug!("We find the file in cache!");
        let (sender, recver) = mpsc::channel(1);
        tokio::spawn(async move {
            let mut offset = 0;
            let blk_size = GRPC_DATA_CHUNK_SIZE;
            while offset < data.len() {
                let end = std::cmp::min(offset + blk_size, data.len());
                let reply = DataChunk {
                    payload: data[offset..end].to_vec(),
                    version,
                };
                sender.send(Ok(reply)).await.unwrap();
                offset = end;
            }
        });
        Ok(Response::new(ReceiverStream::new(recver)))
    }

    async fn write(
        &self,
        request: Request<Streaming<FileToWrite>>,
    ) -> Result<Response<Size>, Status> {
        let mut stream = request.into_inner();
        let mut counter = 0;
        let mut data: Vec<u8> = vec![];
        let mut inode = 0;
        let mut offset = 0;
        while let Some(mut file) = stream.message().await? {
            info!(
                "write[{}](file={}, offset={}, size={})",
                counter,
                file.file,
                file.offset,
                file.data.len()
            );
            counter += 1;
            inode = file.file;
            offset = file.offset;
            data.append(&mut file.data);
        }
        // FIXME: write to tmp file by chunk so we don't eat memory.
        // This way we don't lock the vault when transferring packets on wire.
        let mut vault = self.local().lock().unwrap();
        let size = translate_result(vault.write(inode, offset, &data))?;
        Ok(Response::new(Size { value: size }))
    }

    async fn submit(
        &self,
        request: Request<Streaming<FileToWrite>>,
    ) -> Result<Response<Acceptance>, Status> {
        todo!()
    }

    async fn create(&self, request: Request<FileToCreate>) -> Result<Response<Inode>, Status> {
        let request_inner = request.into_inner();
        info!(
            "create(parent={}, name={}, kind={:?})",
            request_inner.parent,
            request_inner.name.as_str(),
            num2kind(request_inner.kind),
        );
        let mut vault = self.local().lock().unwrap();
        let inode = translate_result(vault.create(
            request_inner.parent,
            request_inner.name.as_str(),
            num2kind(request_inner.kind),
        ))?;
        Ok(Response::new(Inode { value: inode }))
    }
    async fn open(&self, request: Request<FileToOpen>) -> Result<Response<Empty>, Status> {
        let request_inner = request.into_inner();
        let mode = match request_inner.mode {
            0 => OpenMode::R,
            _option => OpenMode::RW,
        };
        info!("open(file={}, mode={:?})", request_inner.file, mode);
        let mut vault = self.local().lock().unwrap();
        translate_result(vault.open(request_inner.file, mode))?;
        Ok(Response::new(Empty {}))
    }
    async fn close(&self, request: Request<Inode>) -> Result<Response<Empty>, Status> {
        let inner = request.into_inner();
        info!("close({})", inner.value);
        let mut vault = self.local().lock().unwrap();
        translate_result(vault.close(inner.value))?;
        Ok(Response::new(Empty {}))
    }
    async fn delete(&self, request: Request<Inode>) -> Result<Response<Empty>, Status> {
        let inner = request.into_inner();
        info!("delete({})", inner.value);
        let mut vault = self.local().lock().unwrap();
        translate_result(vault.delete(inner.value))?;
        Ok(Response::new(Empty {}))
    }
    async fn readdir(&self, request: Request<Inode>) -> Result<Response<DirEntryList>, Status> {
        let inner = request.into_inner();
        info!("readdir({})", inner.value);
        let mut vault = self.local().lock().unwrap();
        let entries = translate_result(vault.readdir(inner.value))?;

        Ok(Response::new(DirEntryList {
            list: entries
                .into_iter()
                .map(|e| FileInfo {
                    inode: e.inode,
                    name: e.name,
                    kind: kind2num(e.kind),
                    size: e.size,
                    atime: e.atime,
                    mtime: e.mtime,
                    version: e.version,
                })
                .collect(),
        }))
    }
}
