// A gRPC server that receives requests and uses local_vault to do the
// actual work.
use async_trait::async_trait;
use tonic::{Request, Response, Status, Streaming};
use crate::local_vault::LocalVault;
use crate::rpc::vault_rpc_server::VaultRpc;
use crate::rpc::{DirEntry,FileToWrite, FileToCreate, FileToRead, Size, FileToOpen, Empty, DirEntryList,Inode, FileInfo, DataChunk};
use crate::types::{Vault,OpenMode,VaultFileType};
use tokio_stream::wrappers::ReceiverStream;
use tokio::sync::mpsc;

struct VaultServer {
    local_vault: LocalVault,
}
fn kind2num(v: VaultFileType)->i32{
    let k = match v {
        VaultFileType::File => 1,
        VaultFileType::Directory => 2, 
    };
    return k;
}
fn num2kind(k:i32)->VaultFileType{
    if k==1{
        return VaultFileType::File;
    }else{
        return VaultFileType::Directory;
    }
}
#[async_trait] 
impl VaultRpc for VaultServer {
    async fn attr(&self,request: Request<Inode>,) -> Result<Response<FileInfo>, Status> {
        let res = self.local_vault.attr(request.into_inner().value);
        match res {
            Ok(v) => {
                Ok(Response::new(FileInfo{ inode: v.inode, name: v.name, kind: kind2num(v.kind), size: v.size }))
            },
            Err(_) => Err(Status::unknown("Function attr in VaultServer failed!")),
        }
    }
    type readStream = ReceiverStream<Result<DataChunk, Status>>;
    async fn read(
        &self,
        request: Request<FileToRead>,
    ) -> Result<Response<Self::readStream>, Status>
    {
        let request_inner = request.into_inner();
        let res = self.local_vault.read(request_inner.file,request_inner.offset,request_inner.size).unwrap();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tx.send(Ok(DataChunk{payload:res.clone()})).await.unwrap();
        });
        Ok(Response::new(ReceiverStream::new(rx)))
    }
    async fn write(
        &self,
        request: Request<Streaming<FileToWrite>>,
    ) -> Result<Response<Size>, Status>
    {
        let mut stream = request.into_inner();
        let mut size = 0;

        while let Some(file) = stream.message().await?{
            let res = self.local_vault.write(file.name,file.offset,&file.data);
            match res {
                Ok(v) => size+=v,
                Err(_) => return Err(Status::unknown("Function write in VaultServer failed!")),
            }
        }
        Ok(Response::new(Size { value: size }))
    }
    async fn create(
        &self,
        request: Request<FileToCreate>,
    ) -> Result<Response<Inode>, Status>
    {
        let request_inner = request.into_inner();
        let res = self.local_vault.create(request_inner.parent,request_inner.name.as_str(),num2kind(request_inner.kind));
        match res {
            Ok(v) => {
                Ok(Response::new(Inode{ value: v }))
            },
            Err(_) => Err(Status::unknown("Function create in VaultServer failed!")),
        }
    }
    async fn open(
        &self,
        request: Request<FileToOpen>,
    ) -> Result<Response<Empty>, Status>
    {
        let request_inner = request.into_inner();
        let mode = match request_inner.mode{
            0=>OpenMode::R,
            _option=>OpenMode::RW,
        };
        let res = self.local_vault.open(request_inner.file,mode);
        match res {
            Ok(v) => {
                Ok(Response::new(Empty{}))
            },
            Err(_) => Err(Status::unknown("Function open in VaultServer failed!")),
        }
    }
    async fn close(
        &self,
        request: Request<Inode>,
    ) -> Result<Response<Empty>, Status>
    {
        let res = self.local_vault.close(request.into_inner().value);
        match res {
            Ok(v) => {
                Ok(Response::new(Empty{}))
            },
            Err(_) => Err(Status::unknown("Function close in VaultServer failed!")),
        }
    }
    async fn delete(
        &self,
        request: Request<Inode>,
    ) -> Result<Response<Empty>, Status>
    {
        let res = self.local_vault.delete(request.into_inner().value);
        match res {
            Ok(v) => {
                Ok(Response::new(Empty{}))
            },
            Err(_) => Err(Status::unknown("Function delete in VaultServer failed!")),
        }
    }
    async fn readdir(
        &self,
        request: Request<Inode>,
    ) -> Result<Response<DirEntryList>, Status>
    {
        let res = self.local_vault.readdir(request.into_inner().value);
        match res {
            Ok(v) => {

                Ok(Response::new(DirEntryList{ list: v.into_iter().map(|e| DirEntry{inode: e.inode, name: e.name, kind: kind2num(e.kind) }).collect()}))
            },
            Err(_) => Err(Status::unknown("Function delete in VaultServer failed!")),
        }
    }
}