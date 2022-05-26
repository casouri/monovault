// A gRPC server that receives requests and uses local_vault to do the
// actual work.
use crate::rpc::vault_rpc_server;
use crate::rpc::vault_rpc_server::VaultRpc;
use crate::rpc::{
    DataChunk, DirEntryList, Empty, FileInfo, FileToCreate, FileToOpen, FileToRead, FileToWrite,
    Inode, Size,
};
use crate::types::{OpenMode, Vault, VaultFileType};
use async_trait::async_trait;
use log::info;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

pub fn run_server(address: &str, local_vault: Arc<Mutex<Box<dyn Vault>>>) {
    let rt = Builder::new_multi_thread().enable_all().build().unwrap();
    let incoming = match rt.block_on(TcpListener::bind(address)) {
        Ok(lis) => tokio_stream::wrappers::TcpListenerStream::new(lis),
        Err(err) => {
            panic!("Error creating incoming channel for server: {}", err);
        }
    };
    let service = vault_rpc_server::VaultRpcServer::new(VaultServer::new(local_vault));
    let server = tonic::transport::Server::builder().add_service(service);
    info!("Server started");
    rt.block_on(server.serve_with_incoming(incoming)).unwrap();
}

pub struct VaultServer {
    local_vault: Arc<Mutex<Box<dyn Vault>>>,
}

fn kind2num(v: VaultFileType) -> i32 {
    let k = match v {
        VaultFileType::File => 1,
        VaultFileType::Directory => 2,
    };
    return k;
}

fn num2kind(k: i32) -> VaultFileType {
    if k == 1 {
        return VaultFileType::File;
    } else {
        return VaultFileType::Directory;
    }
}

impl VaultServer {
    pub fn new(local_vault: Arc<Mutex<Box<dyn Vault>>>) -> VaultServer {
        VaultServer { local_vault }
    }
}

#[async_trait]
impl VaultRpc for VaultServer {
    async fn attr(&self, request: Request<Inode>) -> Result<Response<FileInfo>, Status> {
        let inner = request.into_inner();
        info!("attr({})", inner.value);
        let res = self.local_vault.lock().unwrap().attr(inner.value);
        match res {
            Ok(v) => Ok(Response::new(FileInfo {
                inode: v.inode,
                name: v.name,
                kind: kind2num(v.kind),
                size: v.size,
                atime: v.atime,
                mtime: v.mtime,
                version: v.version,
            })),
            Err(_) => Err(Status::unknown("Function attr in VaultServer failed!")),
        }
    }
    type readStream = ReceiverStream<Result<DataChunk, Status>>;
    async fn read(
        &self,
        request: Request<FileToRead>,
    ) -> Result<Response<Self::readStream>, Status> {
        let request_inner = request.into_inner();
        info!(
            "read(file={}, offset={}, size={})",
            request_inner.file, request_inner.offset, request_inner.size
        );
        let res = self
            .local_vault
            .lock()
            .unwrap()
            .read(request_inner.file, request_inner.offset, request_inner.size)
            .unwrap();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tx.send(Ok(DataChunk {
                payload: res.clone(),
            }))
            .await
            .unwrap();
        });
        Ok(Response::new(ReceiverStream::new(rx)))
    }
    async fn write(
        &self,
        request: Request<Streaming<FileToWrite>>,
    ) -> Result<Response<Size>, Status> {
        let mut stream = request.into_inner();
        let mut size = 0;
        let mut counter = 0;
        while let Some(file) = stream.message().await? {
            info!(
                "write[{}](file={}, offset={}, size={})",
                counter,
                file.file,
                file.offset,
                file.data.len()
            );
            counter += 1;
            let res = self
                .local_vault
                .lock()
                .unwrap()
                .write(file.file, file.offset, &file.data);
            match res {
                Ok(v) => size += v,
                Err(_) => return Err(Status::unknown("Function write in VaultServer failed!")),
            }
        }
        Ok(Response::new(Size { value: size }))
    }
    async fn create(&self, request: Request<FileToCreate>) -> Result<Response<Inode>, Status> {
        let request_inner = request.into_inner();
        info!(
            "create(parent={}, name={}, kind={:?})",
            request_inner.parent,
            request_inner.name.as_str(),
            num2kind(request_inner.kind),
        );
        let res = self.local_vault.lock().unwrap().create(
            request_inner.parent,
            request_inner.name.as_str(),
            num2kind(request_inner.kind),
        );
        match res {
            Ok(v) => Ok(Response::new(Inode { value: v })),
            Err(_) => Err(Status::unknown("Function create in VaultServer failed!")),
        }
    }
    async fn open(&self, request: Request<FileToOpen>) -> Result<Response<Empty>, Status> {
        let request_inner = request.into_inner();
        let mode = match request_inner.mode {
            0 => OpenMode::R,
            _option => OpenMode::RW,
        };
        info!("open(file={}, mode={:?})", request_inner.file, mode);
        let res = self
            .local_vault
            .lock()
            .unwrap()
            .open(request_inner.file, mode);
        match res {
            Ok(v) => Ok(Response::new(Empty {})),
            Err(_) => Err(Status::unknown("Function open in VaultServer failed!")),
        }
    }
    async fn close(&self, request: Request<Inode>) -> Result<Response<Empty>, Status> {
        let inner = request.into_inner();
        info!("close({})", inner.value);
        let res = self.local_vault.lock().unwrap().close(inner.value);
        match res {
            Ok(_) => Ok(Response::new(Empty {})),
            Err(_) => Err(Status::unknown("Function close in VaultServer failed!")),
        }
    }
    async fn delete(&self, request: Request<Inode>) -> Result<Response<Empty>, Status> {
        let inner = request.into_inner();
        info!("delete({})", inner.value);
        let res = self.local_vault.lock().unwrap().delete(inner.value);
        match res {
            Ok(_) => Ok(Response::new(Empty {})),
            Err(_) => Err(Status::unknown("Function delete in VaultServer failed!")),
        }
    }
    async fn readdir(&self, request: Request<Inode>) -> Result<Response<DirEntryList>, Status> {
        let inner = request.into_inner();
        info!("readdir({})", inner.value);
        let res = self.local_vault.lock().unwrap().readdir(inner.value);
        match res {
            Ok(v) => Ok(Response::new(DirEntryList {
                list: v
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
            })),
            Err(_) => Err(Status::unknown("Function delete in VaultServer failed!")),
        }
    }
}
