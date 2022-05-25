#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Empty {
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Size {
    #[prost(uint32, tag="1")]
    pub value: u32,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Inode {
    #[prost(uint64, tag="1")]
    pub value: u64,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileInfo {
    #[prost(uint64, tag="1")]
    pub inode: u64,
    #[prost(string, tag="2")]
    pub name: ::prost::alloc::string::String,
    #[prost(enumeration="VaultFileType", tag="3")]
    pub kind: i32,
    #[prost(uint64, tag="4")]
    pub size: u64,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DirEntry {
    #[prost(uint64, tag="1")]
    pub inode: u64,
    #[prost(string, tag="2")]
    pub name: ::prost::alloc::string::String,
    #[prost(enumeration="VaultFileType", tag="3")]
    pub kind: i32,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DirEntryList {
    #[prost(message, repeated, tag="1")]
    pub list: ::prost::alloc::vec::Vec<DirEntry>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileToRead {
    #[prost(uint64, tag="1")]
    pub file: u64,
    #[prost(int64, tag="2")]
    pub offset: i64,
    #[prost(uint32, tag="3")]
    pub size: u32,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileToWrite {
    #[prost(uint64, tag="1")]
    pub name: u64,
    #[prost(int64, tag="2")]
    pub offset: i64,
    #[prost(bytes="vec", tag="3")]
    pub data: ::prost::alloc::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileToCreate {
    #[prost(uint64, tag="1")]
    pub parent: u64,
    #[prost(string, tag="2")]
    pub name: ::prost::alloc::string::String,
    #[prost(enumeration="VaultFileType", tag="3")]
    pub kind: i32,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileToOpen {
    #[prost(uint64, tag="1")]
    pub file: u64,
    #[prost(enumeration="file_to_open::OpenMode", tag="2")]
    pub mode: i32,
}
/// Nested message and enum types in `FileToOpen`.
pub mod file_to_open {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum OpenMode {
        R = 0,
        Rw = 1,
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DataChunk {
    #[prost(bytes="vec", tag="1")]
    pub payload: ::prost::alloc::vec::Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum VaultFileType {
    File = 0,
    Directory = 1,
}
/// Generated client implementations.
pub mod vault_rpc_client {
    #![allow(unused_variables, dead_code, missing_docs, clippy::let_unit_value)]
    use tonic::codegen::*;
    #[derive(Debug, Clone)]
    pub struct VaultRpcClient<T> {
        inner: tonic::client::Grpc<T>,
    }
    impl VaultRpcClient<tonic::transport::Channel> {
        /// Attempt to create a new client by connecting to a given endpoint.
        pub async fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
        where
            D: std::convert::TryInto<tonic::transport::Endpoint>,
            D::Error: Into<StdError>,
        {
            let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
            Ok(Self::new(conn))
        }
    }
    impl<T> VaultRpcClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::BoxBody>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + Send,
    {
        pub fn new(inner: T) -> Self {
            let inner = tonic::client::Grpc::new(inner);
            Self { inner }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> VaultRpcClient<InterceptedService<T, F>>
        where
            F: tonic::service::Interceptor,
            T::ResponseBody: Default,
            T: tonic::codegen::Service<
                http::Request<tonic::body::BoxBody>,
                Response = http::Response<
                    <T as tonic::client::GrpcService<tonic::body::BoxBody>>::ResponseBody,
                >,
            >,
            <T as tonic::codegen::Service<
                http::Request<tonic::body::BoxBody>,
            >>::Error: Into<StdError> + Send + Sync,
        {
            VaultRpcClient::new(InterceptedService::new(inner, interceptor))
        }
        /// Compress requests with `gzip`.
        ///
        /// This requires the server to support it otherwise it might respond with an
        /// error.
        #[must_use]
        pub fn send_gzip(mut self) -> Self {
            self.inner = self.inner.send_gzip();
            self
        }
        /// Enable decompressing responses with `gzip`.
        #[must_use]
        pub fn accept_gzip(mut self) -> Self {
            self.inner = self.inner.accept_gzip();
            self
        }
        pub async fn attr(
            &mut self,
            request: impl tonic::IntoRequest<super::Inode>,
        ) -> Result<tonic::Response<super::FileInfo>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/rpc.VaultRPC/attr");
            self.inner.unary(request.into_request(), path, codec).await
        }
        pub async fn read(
            &mut self,
            request: impl tonic::IntoRequest<super::FileToRead>,
        ) -> Result<
                tonic::Response<tonic::codec::Streaming<super::DataChunk>>,
                tonic::Status,
            > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/rpc.VaultRPC/read");
            self.inner.server_streaming(request.into_request(), path, codec).await
        }
        pub async fn write(
            &mut self,
            request: impl tonic::IntoStreamingRequest<Message = super::FileToWrite>,
        ) -> Result<tonic::Response<super::Size>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/rpc.VaultRPC/write");
            self.inner
                .client_streaming(request.into_streaming_request(), path, codec)
                .await
        }
        pub async fn create(
            &mut self,
            request: impl tonic::IntoRequest<super::FileToCreate>,
        ) -> Result<tonic::Response<super::Inode>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/rpc.VaultRPC/create");
            self.inner.unary(request.into_request(), path, codec).await
        }
        pub async fn open(
            &mut self,
            request: impl tonic::IntoRequest<super::FileToOpen>,
        ) -> Result<tonic::Response<super::Empty>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/rpc.VaultRPC/open");
            self.inner.unary(request.into_request(), path, codec).await
        }
        pub async fn close(
            &mut self,
            request: impl tonic::IntoRequest<super::Inode>,
        ) -> Result<tonic::Response<super::Empty>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/rpc.VaultRPC/close");
            self.inner.unary(request.into_request(), path, codec).await
        }
        pub async fn delete(
            &mut self,
            request: impl tonic::IntoRequest<super::Inode>,
        ) -> Result<tonic::Response<super::Empty>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/rpc.VaultRPC/delete");
            self.inner.unary(request.into_request(), path, codec).await
        }
        pub async fn readdir(
            &mut self,
            request: impl tonic::IntoRequest<super::Inode>,
        ) -> Result<tonic::Response<super::DirEntryList>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/rpc.VaultRPC/readdir");
            self.inner.unary(request.into_request(), path, codec).await
        }
    }
}
/// Generated server implementations.
pub mod vault_rpc_server {
    #![allow(unused_variables, dead_code, missing_docs, clippy::let_unit_value)]
    use tonic::codegen::*;
    ///Generated trait containing gRPC methods that should be implemented for use with VaultRpcServer.
    #[async_trait]
    pub trait VaultRpc: Send + Sync + 'static {
        async fn attr(
            &self,
            request: tonic::Request<super::Inode>,
        ) -> Result<tonic::Response<super::FileInfo>, tonic::Status>;
        ///Server streaming response type for the read method.
        type readStream: futures_core::Stream<
                Item = Result<super::DataChunk, tonic::Status>,
            >
            + Send
            + 'static;
        async fn read(
            &self,
            request: tonic::Request<super::FileToRead>,
        ) -> Result<tonic::Response<Self::readStream>, tonic::Status>;
        async fn write(
            &self,
            request: tonic::Request<tonic::Streaming<super::FileToWrite>>,
        ) -> Result<tonic::Response<super::Size>, tonic::Status>;
        async fn create(
            &self,
            request: tonic::Request<super::FileToCreate>,
        ) -> Result<tonic::Response<super::Inode>, tonic::Status>;
        async fn open(
            &self,
            request: tonic::Request<super::FileToOpen>,
        ) -> Result<tonic::Response<super::Empty>, tonic::Status>;
        async fn close(
            &self,
            request: tonic::Request<super::Inode>,
        ) -> Result<tonic::Response<super::Empty>, tonic::Status>;
        async fn delete(
            &self,
            request: tonic::Request<super::Inode>,
        ) -> Result<tonic::Response<super::Empty>, tonic::Status>;
        async fn readdir(
            &self,
            request: tonic::Request<super::Inode>,
        ) -> Result<tonic::Response<super::DirEntryList>, tonic::Status>;
    }
    #[derive(Debug)]
    pub struct VaultRpcServer<T: VaultRpc> {
        inner: _Inner<T>,
        accept_compression_encodings: (),
        send_compression_encodings: (),
    }
    struct _Inner<T>(Arc<T>);
    impl<T: VaultRpc> VaultRpcServer<T> {
        pub fn new(inner: T) -> Self {
            Self::from_arc(Arc::new(inner))
        }
        pub fn from_arc(inner: Arc<T>) -> Self {
            let inner = _Inner(inner);
            Self {
                inner,
                accept_compression_encodings: Default::default(),
                send_compression_encodings: Default::default(),
            }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> InterceptedService<Self, F>
        where
            F: tonic::service::Interceptor,
        {
            InterceptedService::new(Self::new(inner), interceptor)
        }
    }
    impl<T, B> tonic::codegen::Service<http::Request<B>> for VaultRpcServer<T>
    where
        T: VaultRpc,
        B: Body + Send + 'static,
        B::Error: Into<StdError> + Send + 'static,
    {
        type Response = http::Response<tonic::body::BoxBody>;
        type Error = std::convert::Infallible;
        type Future = BoxFuture<Self::Response, Self::Error>;
        fn poll_ready(
            &mut self,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, req: http::Request<B>) -> Self::Future {
            let inner = self.inner.clone();
            match req.uri().path() {
                "/rpc.VaultRPC/attr" => {
                    #[allow(non_camel_case_types)]
                    struct attrSvc<T: VaultRpc>(pub Arc<T>);
                    impl<T: VaultRpc> tonic::server::UnaryService<super::Inode>
                    for attrSvc<T> {
                        type Response = super::FileInfo;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::Inode>,
                        ) -> Self::Future {
                            let inner = self.0.clone();
                            let fut = async move { (*inner).attr(request).await };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let inner = inner.0;
                        let method = attrSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/rpc.VaultRPC/read" => {
                    #[allow(non_camel_case_types)]
                    struct readSvc<T: VaultRpc>(pub Arc<T>);
                    impl<
                        T: VaultRpc,
                    > tonic::server::ServerStreamingService<super::FileToRead>
                    for readSvc<T> {
                        type Response = super::DataChunk;
                        type ResponseStream = T::readStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::FileToRead>,
                        ) -> Self::Future {
                            let inner = self.0.clone();
                            let fut = async move { (*inner).read(request).await };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let inner = inner.0;
                        let method = readSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            );
                        let res = grpc.server_streaming(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/rpc.VaultRPC/write" => {
                    #[allow(non_camel_case_types)]
                    struct writeSvc<T: VaultRpc>(pub Arc<T>);
                    impl<
                        T: VaultRpc,
                    > tonic::server::ClientStreamingService<super::FileToWrite>
                    for writeSvc<T> {
                        type Response = super::Size;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<tonic::Streaming<super::FileToWrite>>,
                        ) -> Self::Future {
                            let inner = self.0.clone();
                            let fut = async move { (*inner).write(request).await };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let inner = inner.0;
                        let method = writeSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            );
                        let res = grpc.client_streaming(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/rpc.VaultRPC/create" => {
                    #[allow(non_camel_case_types)]
                    struct createSvc<T: VaultRpc>(pub Arc<T>);
                    impl<T: VaultRpc> tonic::server::UnaryService<super::FileToCreate>
                    for createSvc<T> {
                        type Response = super::Inode;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::FileToCreate>,
                        ) -> Self::Future {
                            let inner = self.0.clone();
                            let fut = async move { (*inner).create(request).await };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let inner = inner.0;
                        let method = createSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/rpc.VaultRPC/open" => {
                    #[allow(non_camel_case_types)]
                    struct openSvc<T: VaultRpc>(pub Arc<T>);
                    impl<T: VaultRpc> tonic::server::UnaryService<super::FileToOpen>
                    for openSvc<T> {
                        type Response = super::Empty;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::FileToOpen>,
                        ) -> Self::Future {
                            let inner = self.0.clone();
                            let fut = async move { (*inner).open(request).await };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let inner = inner.0;
                        let method = openSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/rpc.VaultRPC/close" => {
                    #[allow(non_camel_case_types)]
                    struct closeSvc<T: VaultRpc>(pub Arc<T>);
                    impl<T: VaultRpc> tonic::server::UnaryService<super::Inode>
                    for closeSvc<T> {
                        type Response = super::Empty;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::Inode>,
                        ) -> Self::Future {
                            let inner = self.0.clone();
                            let fut = async move { (*inner).close(request).await };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let inner = inner.0;
                        let method = closeSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/rpc.VaultRPC/delete" => {
                    #[allow(non_camel_case_types)]
                    struct deleteSvc<T: VaultRpc>(pub Arc<T>);
                    impl<T: VaultRpc> tonic::server::UnaryService<super::Inode>
                    for deleteSvc<T> {
                        type Response = super::Empty;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::Inode>,
                        ) -> Self::Future {
                            let inner = self.0.clone();
                            let fut = async move { (*inner).delete(request).await };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let inner = inner.0;
                        let method = deleteSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/rpc.VaultRPC/readdir" => {
                    #[allow(non_camel_case_types)]
                    struct readdirSvc<T: VaultRpc>(pub Arc<T>);
                    impl<T: VaultRpc> tonic::server::UnaryService<super::Inode>
                    for readdirSvc<T> {
                        type Response = super::DirEntryList;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::Inode>,
                        ) -> Self::Future {
                            let inner = self.0.clone();
                            let fut = async move { (*inner).readdir(request).await };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let inner = inner.0;
                        let method = readdirSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                _ => {
                    Box::pin(async move {
                        Ok(
                            http::Response::builder()
                                .status(200)
                                .header("grpc-status", "12")
                                .header("content-type", "application/grpc")
                                .body(empty_body())
                                .unwrap(),
                        )
                    })
                }
            }
        }
    }
    impl<T: VaultRpc> Clone for VaultRpcServer<T> {
        fn clone(&self) -> Self {
            let inner = self.inner.clone();
            Self {
                inner,
                accept_compression_encodings: self.accept_compression_encodings,
                send_compression_encodings: self.send_compression_encodings,
            }
        }
    }
    impl<T: VaultRpc> Clone for _Inner<T> {
        fn clone(&self) -> Self {
            Self(self.0.clone())
        }
    }
    impl<T: std::fmt::Debug> std::fmt::Debug for _Inner<T> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{:?}", self.0)
        }
    }
    impl<T: VaultRpc> tonic::transport::NamedService for VaultRpcServer<T> {
        const NAME: &'static str = "rpc.VaultRPC";
    }
}
