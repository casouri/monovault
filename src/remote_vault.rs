// Basically a gRPC client that makes requests to remote vault servers.

use crate::rpc;
use crate::rpc::vault_rpc_client::VaultRpcClient;
use crate::rpc::FileToWrite;
use crate::types::VaultFileType::Directory;
use crate::types::VaultFileType::File;
use crate::types::*;
use futures_util::stream;
use std::sync::Mutex;
use tokio::runtime::{Builder, Runtime};
use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tonic::Request;

#[derive(Debug)]
pub struct RemoteVault {
    rt: Mutex<Runtime>,
    client: Mutex<VaultRpcClient<Channel>>,
    name: String,
}

impl RemoteVault {
    pub fn new(addr: String, name: &str) -> VaultResult<RemoteVault> {
        let rt = Builder::new_multi_thread().enable_all().build().unwrap();
        let client = rt.block_on(VaultRpcClient::connect(addr))?;
        return Ok(RemoteVault {
            rt: Mutex::new(rt),
            client: Mutex::new(client),
            name: name.to_string(),
        });
    }
}

impl Vault for RemoteVault {
    fn name(&self) -> String {
        return self.name.clone();
    }

    fn attr(&mut self, file: Inode) -> VaultResult<FileInfo> {
        let rt = self.rt.lock().unwrap();
        let mut client = self.client.lock().unwrap();
        let response = rt.block_on(client.attr(rpc::Inode { value: file }));
        match response {
            Ok(value) => {
                let v = value.into_inner();
                Ok(FileInfo {
                    inode: v.inode,
                    name: v.name.to_string(),
                    kind: if v.kind == 0 {
                        VaultFileType::File
                    } else {
                        VaultFileType::Directory
                    },
                    size: v.size,
                    atime: v.atime,
                    mtime: v.mtime,
                    version: v.version,
                })
            }
            Err(_) => {
                // TODO: Status Code
                Err(VaultError::FileNotExist(file))
            }
        }
    }

    fn read(&mut self, file: Inode, offset: i64, size: u32) -> VaultResult<Vec<u8>> {
        let mut result: Vec<u8> = Vec::new();
        let rt = self.rt.lock().unwrap();
        let mut client = self.client.lock().unwrap();
        let response = rt.block_on(client.read(rpc::FileToRead { file, offset, size }));
        match response {
            Ok(value) => {
                let mut stream = value.into_inner();
                while let Some(received) = rt.block_on(stream.next()) {
                    match received {
                        Ok(value) => {
                            result.extend(&value.payload);
                        }
                        Err(_) => {
                            // TODO: which one to return?
                            // return Err(VaultError::FileNotExist(file)); // TODO: status code
                            return Ok(result);
                        }
                    }
                }
                return Ok(result);
            }
            Err(_) => {
                // TODO: status code
                Err(VaultError::FileNotExist(file))
            }
        }
    }

    fn write(&mut self, file: Inode, offset: i64, data: &[u8]) -> VaultResult<u32> {
        let rt = self.rt.lock().unwrap();
        let mut client = self.client.lock().unwrap();
        let mut file2write = vec![];
        file2write.push(FileToWrite {
            name: file,
            offset,
            data: data.to_vec(),
        });
        let request = Request::new(stream::iter(file2write));
        let response = rt.block_on(client.write(request)).unwrap();
        Ok(response.into_inner().value)
    }

    fn create(&mut self, parent: Inode, name: &str, kind: VaultFileType) -> VaultResult<Inode> {
        let rt = self.rt.lock().unwrap();
        let mut client = self.client.lock().unwrap();
        let mut request = rpc::FileToCreate {
            parent,
            name: name.to_string(),
            kind: 1, // File = 0, Directory = 1,
        };
        if matches!(kind, File) {
            request.kind = 0;
        }
        let response = rt.block_on(client.create(request)).unwrap().into_inner();
        return Ok(response.value);
    }

    fn open(&mut self, file: Inode, mode: OpenMode) -> VaultResult<()> {
        let rt = self.rt.lock().unwrap();
        let mut client = self.client.lock().unwrap();
        let mut request = rpc::FileToOpen {
            file,
            mode: 1, // R = 0, Rw = 1,
        };
        if matches!(mode, OpenMode::R) {
            request.mode = 0;
        }
        let _ = rt.block_on(client.open(request)).unwrap().into_inner();
        return Ok(());
    }

    fn close(&mut self, file: Inode) -> VaultResult<()> {
        let rt = self.rt.lock().unwrap();
        let mut client = self.client.lock().unwrap();
        let _ = rt
            .block_on(client.close(rpc::Inode { value: file }))
            .unwrap()
            .into_inner();
        return Ok(());
    }

    fn delete(&mut self, file: Inode) -> VaultResult<()> {
        let rt = self.rt.lock().unwrap();
        let mut client = self.client.lock().unwrap();
        let _ = rt
            .block_on(client.delete(rpc::Inode { value: file }))
            .unwrap()
            .into_inner();
        return Ok(());
    }

    fn readdir(&mut self, dir: Inode) -> VaultResult<Vec<FileInfo>> {
        let rt = self.rt.lock().unwrap();
        let mut client = self.client.lock().unwrap();
        let response = rt
            .block_on(client.readdir(rpc::Inode { value: dir }))
            .unwrap()
            .into_inner()
            .list;
        let result: Vec<FileInfo> = response
            .iter()
            .map(|info| FileInfo {
                inode: info.inode,
                name: info.name.clone(),
                kind: if info.kind == 0 { File } else { Directory },
                size: info.size,
                atime: info.atime,
                mtime: info.mtime,
                version: info.version,
            })
            .collect();
        return Ok(result);
    }
}
