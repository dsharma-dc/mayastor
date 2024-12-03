use crate::{
    bdev_api::{bdev_create, bdev_destroy, BdevError},
    core,
    core::{CoreError, NvmfShareProps, Protocol, Share},
    grpc::{rpc_submit, GrpcResult},
};
use io_engine_api::v1::bdev::{
    Bdev, BdevRpc, BdevShareRequest, BdevShareResponse, BdevUnshareRequest, CreateBdevRequest,
    CreateBdevResponse, DestroyBdevRequest, ListBdevOptions, ListBdevResponse,
};
use std::{convert::TryFrom, pin::Pin};
use tonic::{Request, Response, Status};
use url::Url;

impl<T> From<core::Bdev<T>> for Bdev
where
    T: spdk_rs::BdevOps,
{
    fn from(b: core::Bdev<T>) -> Self {
        Self {
            name: b.name().to_string(),
            uuid: b.uuid_as_string(),
            num_blocks: b.num_blocks(),
            blk_size: b.block_len(),
            claimed: b.is_claimed(),
            claimed_by: b
                .first_claim_module_name()
                .unwrap_or_else(|| "Orphaned".into()),
            aliases: b.aliases().join(","),
            product_name: b.product_name().to_string(),
            share_uri: b.share_uri().unwrap_or_else(|| "".into()),
            uri: Url::try_from(b).map_or("".into(), |u| u.to_string()),
        }
    }
}

/// RPC service for spdk bdev operations
#[derive(Debug)]
pub struct BdevService {}

impl BdevService {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for BdevService {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl BdevRpc for BdevService {
    #[tracing::instrument(skip(self))]
    async fn list(&self, request: Request<ListBdevOptions>) -> GrpcResult<ListBdevResponse> {
        let rx = rpc_submit::<_, _, BdevError>(async {
            let mut bdevs = Vec::new();
            let args = request.into_inner();
            if let Some(name) = args.name {
                if let Some(bdev) = core::UntypedBdev::lookup_by_name(&name) {
                    bdevs.push(bdev.into());
                }
            } else if let Some(bdev) = core::UntypedBdev::bdev_first() {
                bdev.into_iter().for_each(|bdev| bdevs.push(bdev.into()))
            }

            Ok(ListBdevResponse { bdevs })
        })?;

        rx.await
            .map_err(|_| Status::cancelled("cancelled"))?
            .map_err(Status::from)
            .map(Response::new)
    }

    #[tracing::instrument(skip(self))]
    async fn create(
        &self,
        request: Request<CreateBdevRequest>,
    ) -> Result<Response<CreateBdevResponse>, Status> {
        let uri = request.into_inner().uri;

        let rx = rpc_submit(async move {
            let name = bdev_create(&uri).await?;

            if let Some(bdev) = core::UntypedBdev::lookup_by_name(&name) {
                Ok(Response::new(CreateBdevResponse {
                    bdev: Some(bdev.into()),
                }))
            } else {
                Err(BdevError::BdevNotFound { name })
            }
        })?;

        rx.await
            .map_err(|_| Status::cancelled("cancelled"))?
            .map_err(Status::from)
    }

    #[tracing::instrument(skip(self))]
    async fn destroy(&self, request: Request<DestroyBdevRequest>) -> GrpcResult<()> {
        let uri = request.into_inner().uri;

        let rx = rpc_submit(async move { bdev_destroy(&uri).await })?;

        rx.await
            .map_err(|_| Status::cancelled("cancelled"))?
            .map_err(Status::from)
            .map(|_| Ok(Response::new(())))?
    }

    #[tracing::instrument(skip(self))]
    async fn share(
        &self,
        request: Request<BdevShareRequest>,
    ) -> Result<Response<BdevShareResponse>, Status> {
        let r = request.into_inner();
        let bdev_name = r.name.clone();
        let protocol = r.protocol;

        let rx = match Protocol::try_from(protocol) {
            Ok(Protocol::Nvmf) => rpc_submit::<_, Bdev, CoreError>(async move {
                let mut bdev = core::UntypedBdev::get_by_name(&bdev_name)?;
                let props = NvmfShareProps::new().with_allowed_hosts(r.allowed_hosts);
                Pin::new(&mut bdev).share_nvmf(Some(props)).await?;
                let bdev = core::UntypedBdev::get_by_name(&bdev_name)?;
                Ok(bdev.into())
            }),

            _ => return Err(Status::invalid_argument(protocol.to_string())),
        }?;

        rx.await
            .map_err(|_| Status::cancelled("cancelled"))?
            .map_err(|e| match e {
                CoreError::BdevNotFound { name } => Status::not_found(name),
                e => Status::internal(e.to_string()),
            })
            .map(|bdev| Ok(Response::new(BdevShareResponse { bdev: Some(bdev) })))?
    }

    #[tracing::instrument(skip(self))]
    async fn unshare(&self, request: Request<BdevUnshareRequest>) -> GrpcResult<()> {
        let rx = rpc_submit::<_, _, CoreError>(async {
            let name = request.into_inner().name;
            if let Some(mut bdev) = core::UntypedBdev::lookup_by_name(&name) {
                let _ = Pin::new(&mut bdev).unshare().await?;
            }
            Ok(())
        })?;

        rx.await
            .map_err(|_| Status::cancelled("cancelled"))?
            .map_err(Status::from)
            .map(Response::new)
    }
}
