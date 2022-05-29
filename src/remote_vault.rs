/// Basically a gRPC client that makes requests to remote vault
/// servers. This does not mask network error into FileNotFind errors:
/// caching remote uses this as a backend.
use crate::rpc;
use crate::rpc::vault_rpc_client::VaultRpcClient;
use crate::rpc::FileToWrite;
use crate::types::*;
use log::{debug, info};
use tokio::runtime::{Builder, Runtime};
use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tonic::{Request, Status};

#[derive(Debug)]
pub struct RemoteVault {
    rt: Runtime,
    addr: String,
    client: Option<VaultRpcClient<Channel>>,
    name: String,
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

impl RemoteVault {
    pub fn new(addr: &str, name: &str) -> VaultResult<RemoteVault> {
        let rt = Builder::new_multi_thread().enable_all().build().unwrap();
        return Ok(RemoteVault {
            rt,
            addr: addr.to_string(),
            client: None,
            name: name.to_string(),
        });
    }

    fn get_client(&mut self) -> VaultResult<()> {
        let addr = self.addr.clone();
        match &self.client {
            Some(_) => Ok(()),
            None => {
                self.client = Some(self.rt.block_on(VaultRpcClient::connect(addr.clone()))?);
                info!("Connected to {}", addr);
                Ok(())
            }
        }
    }
}

struct WriteIterator {
    file: u64,
    data: Vec<u8>,
    offset: usize,
    block_size: usize,
}

impl WriteIterator {
    // TODO: Avoid copying.
    fn new(file: u64, data: &[u8], offset: usize, block_size: usize) -> WriteIterator {
        WriteIterator {
            file,
            data: data.to_vec(),
            offset,
            block_size,
        }
    }
}

impl Iterator for WriteIterator {
    type Item = FileToWrite;

    fn next(&mut self) -> Option<Self::Item> {
        debug!(
            "write.iter.next(offset={}, blk_size={}, len={})",
            self.offset,
            self.block_size,
            self.data.len()
        );
        if self.offset < self.data.len() {
            let end = std::cmp::min(self.offset + self.block_size, self.data.len());
            let stuff = FileToWrite {
                file: self.file,
                offset: self.offset as i64,
                data: self.data[self.offset..end].to_vec(),
            };
            self.offset = end;
            Some(stuff)
        } else {
            None
        }
    }
}

fn translate_result<T>(res: Result<T, Status>) -> VaultResult<T> {
    match res {
        Ok(val) => Ok(val),
        Err(status) => Err(unpack_status(status)),
    }
}

fn unpack_status(status: Status) -> VaultError {
    if status.code() == tonic::Code::Unknown {
        let compressed: CompressedError = serde_json::from_str(status.message()).unwrap();
        let err: VaultError = compressed.into();
        err
    } else {
        VaultError::RemoteError("unkown".to_string())
    }
}

impl Vault for RemoteVault {
    fn name(&self) -> String {
        return self.name.clone();
    }

    fn attr(&mut self, file: Inode) -> VaultResult<FileInfo> {
        debug!("attr({})", file);
        self.get_client()?;
        let client = self.client.as_mut().unwrap();
        let value = translate_result(self.rt.block_on(client.attr(rpc::Inode { value: file })))?;
        let v = value.into_inner();
        Ok(FileInfo {
            inode: v.inode,
            name: v.name.to_string(),
            kind: num2kind(v.kind),
            size: v.size,
            atime: v.atime,
            mtime: v.mtime,
            version: v.version,
        })
    }

    fn read(&mut self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>> {
        info!("read(file={}, offset={}, size={})", file, offset, size);
        let mut result: Vec<u8> = Vec::new();
        self.get_client()?;
        let client = self.client.as_mut().unwrap();
        let value = translate_result(self.rt.block_on(client.read(rpc::FileToRead {
            file,
            offset,
            size,
        })))?;
        let mut stream = value.into_inner();
        while let Some(received) = self.rt.block_on(stream.next()) {
            let value = translate_result(received)?;
            result.extend(&value.payload);
        }
        return Ok(result);
    }

    fn write(&mut self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32> {
        info!(
            "write(file={}, offset={}, size={})",
            file,
            offset,
            data.len()
        );
        self.get_client()?;
        let client = self.client.as_mut().unwrap();
        let request = Request::new(tokio_stream::iter(WriteIterator::new(
            file,
            data,
            offset as usize,
            GRPC_DATA_CHUNK_SIZE,
        )));
        let response = translate_result(self.rt.block_on(client.write(request)))?;
        Ok(response.into_inner().value)
    }

    fn create(&mut self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        info!("create(parent={}, name={}, kind={:?})", parent, name, kind);
        self.get_client()?;
        let client = self.client.as_mut().unwrap();
        let request = rpc::FileToCreate {
            parent,
            name: name.to_string(),
            kind: kind2num(kind),
        };
        let response = translate_result(self.rt.block_on(client.create(request)))?.into_inner();
        return Ok(response.value);
    }

    fn open(&mut self, file: Inode, mode: OpenMode) -> VaultResult<()> {
        info!("open(file={}, mode={:?})", file, mode);
        self.get_client()?;
        let client = self.client.as_mut().unwrap();
        let mut request = rpc::FileToOpen {
            file,
            mode: 1, // R = 0, RW = 1,
        };
        if matches!(mode, OpenMode::R) {
            request.mode = 0;
        }
        translate_result(self.rt.block_on(client.open(request)))?;
        return Ok(());
    }

    fn close(&mut self, file: Inode) -> VaultResult<()> {
        info!("close({})", file);
        self.get_client()?;
        let client = self.client.as_mut().unwrap();
        translate_result(self.rt.block_on(client.close(rpc::Inode { value: file })))?;

        return Ok(());
    }

    fn delete(&mut self, file: Inode) -> VaultResult<()> {
        info!("delete({})", file);
        self.get_client()?;
        let client = self.client.as_mut().unwrap();
        translate_result(self.rt.block_on(client.delete(rpc::Inode { value: file })))?;
        return Ok(());
    }

    fn readdir(&mut self, dir: Inode) -> VaultResult<Vec<FileInfo>> {
        debug!("readdir({})", dir);
        self.get_client()?;
        let client = self.client.as_mut().unwrap();
        let response =
            translate_result(self.rt.block_on(client.readdir(rpc::Inode { value: dir })))?
                .into_inner()
                .list;
        let result: Vec<FileInfo> = response
            .iter()
            .map(|info| FileInfo {
                inode: info.inode,
                name: info.name.clone(),
                kind: num2kind(info.kind),
                size: info.size,
                atime: info.atime,
                mtime: info.mtime,
                version: info.version,
            })
            .collect();
        return Ok(result);
    }
}
