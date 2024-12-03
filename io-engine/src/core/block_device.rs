use super::{CoreError, DeviceEventSink, IoCompletionStatus, IoType, SnapshotParams};

use spdk_rs::{DmaBuf, DmaError, IoVec};

use async_trait::async_trait;
use futures::channel::oneshot;
use merge::Merge;
use nix::errno::Errno;
use spdk_rs::ffihelper::{cb_arg, done_cb};
use std::os::raw::c_void;
use uuid::Uuid;

/// Structure representing Bdev Io Stats.
#[derive(Debug, Default, Clone, Copy, Merge)]
pub struct BlockDeviceIoStats {
    #[merge(strategy = merge::num::saturating_add)]
    pub num_read_ops: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub num_write_ops: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub bytes_read: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub bytes_written: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub num_unmap_ops: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub bytes_unmapped: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub read_latency_ticks: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub write_latency_ticks: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub unmap_latency_ticks: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub max_read_latency_ticks: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub min_read_latency_ticks: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub max_write_latency_ticks: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub min_write_latency_ticks: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub max_unmap_latency_ticks: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub min_unmap_latency_ticks: u64,
    #[merge(strategy = merge::num::saturating_add)]
    pub tick_rate: u64,
}

/// Core trait that represents a block device.
/// TODO: Add text.
#[async_trait(?Send)]
pub trait BlockDevice {
    /// Returns total size in bytes of the device.
    fn size_in_bytes(&self) -> u64;

    /// Returns the size of a block of the underlying device
    fn block_len(&self) -> u64;

    /// Returns number of blocks for the device.
    fn num_blocks(&self) -> u64;

    /// Returns the UUID of the device.
    fn uuid(&self) -> Uuid;

    /// Returns configured product name for the device.
    fn product_name(&self) -> String;

    /// Returns the name of driver module for the device.
    fn driver_name(&self) -> String;

    /// Returns the name of the device.
    fn device_name(&self) -> String;

    /// Returns aligment of the device.
    fn alignment(&self) -> u64;

    /// Checks whether target I/O type is supported by the device.
    fn io_type_supported(&self, io_type: IoType) -> bool;

    /// Obtains I/O statistics for the device.
    async fn io_stats(&self) -> Result<BlockDeviceIoStats, CoreError>;

    /// Open device and obtain a descriptor.
    fn open(&self, read_write: bool) -> Result<Box<dyn BlockDeviceDescriptor>, CoreError>;

    /// Obtain I/O controller for device.
    fn get_io_controller(&self) -> Option<Box<dyn DeviceIoController>>;

    /// Register device event listener.
    fn add_event_listener(&self, listener: DeviceEventSink) -> Result<(), CoreError>;
}

/// Core trait that represents a descriptor for an opened block device.
/// TODO: Add text.
#[async_trait(?Send)]
pub trait BlockDeviceDescriptor {
    /// TODO
    fn get_device(&self) -> Box<dyn BlockDevice>;

    /// TODO
    fn device_name(&self) -> String;

    /// Consumes BlockDeviceDescriptor and returns a BlockDeviceHandle.
    fn into_handle(self: Box<Self>) -> Result<Box<dyn BlockDeviceHandle>, CoreError>;

    /// Returns a BlockDeviceHandle for this descriptor without consuming it.
    fn get_io_handle(&self) -> Result<Box<dyn BlockDeviceHandle>, CoreError>;

    /// TODO
    fn unclaim(&self);

    /// TODO
    async fn get_io_handle_nonblock(&self) -> Result<Box<dyn BlockDeviceHandle>, CoreError>;
}

/// TODO
pub type IoCompletionCallbackArg = *mut c_void;

/// TODO
pub type IoCompletionCallback =
    fn(&dyn BlockDevice, IoCompletionStatus, IoCompletionCallbackArg) -> ();

/// TODO
pub type OpCompletionCallbackArg = *mut c_void;

/// TODO
pub type OpCompletionCallback = fn(bool, OpCompletionCallbackArg) -> ();

/// Read options.
#[derive(Default, Debug, Copy, Clone)]
pub enum ReadOptions {
    /// Normal read operation.
    #[default]
    None,
    /// Fail when reading an unwritten block of a thin-provisioned device.
    UnwrittenFail,
    /// Fail when reading an unwritten block of a thin-provisioned device.
    CurrentUnwrittenFail,
}

/// Core trait that represents a device I/O handle.
/// TODO: Add text.
#[async_trait(?Send)]
pub trait BlockDeviceHandle {
    /// TODO
    fn get_device(&self) -> &dyn BlockDevice;

    /// TODO
    fn dma_malloc(&self, size: u64) -> Result<DmaBuf, DmaError>;

    /// TODO
    #[deprecated(note = "use read_buf_blocks_async()")]
    async fn read_at(&self, offset: u64, buffer: &mut DmaBuf) -> Result<u64, CoreError>;

    /// TODO
    #[deprecated(note = "use write_buf_blocks_async()")]
    async fn write_at(&self, offset: u64, buffer: &DmaBuf) -> Result<u64, CoreError>;

    /// Reads the given number of blocks into the list of buffers from the
    /// device, starting at the given offset.
    ///
    /// The caller must ensure that the number of blocks to read is equal to
    /// the total size of `iovs` buffer list.
    ///
    /// The given completion callback is called when the operation finishes.
    /// This method may return error immediately in the case operation dispatch
    /// fails.
    fn readv_blocks(
        &self,
        iovs: &mut [IoVec],
        offset_blocks: u64,
        num_blocks: u64,
        opts: ReadOptions,
        cb: IoCompletionCallback,
        cb_arg: IoCompletionCallbackArg,
    ) -> Result<(), CoreError>;

    /// Reads the given number of blocks into the list of buffers from the
    /// device, starting at the given offset.
    ///
    /// The caller must ensure that the number of blocks to read is equal to
    /// the total size of `iovs` buffer list.
    ///
    /// Operation is performed asynchronously; I/O completion status is wrapped
    /// into `CoreError::ReadFailed` in the case of failure.
    async fn readv_blocks_async(
        &self,
        iovs: &mut [IoVec],
        offset_blocks: u64,
        num_blocks: u64,
        opts: ReadOptions,
    ) -> Result<(), CoreError> {
        let (s, r) = oneshot::channel::<IoCompletionStatus>();

        self.readv_blocks(
            iovs,
            offset_blocks,
            num_blocks,
            opts,
            block_device_io_completion,
            cb_arg(s),
        )?;

        match r.await.expect("Failed awaiting at readv_blocks()") {
            IoCompletionStatus::Success => Ok(()),
            status => Err(CoreError::ReadFailed {
                status,
                offset: offset_blocks,
                len: num_blocks,
            }),
        }
    }

    /// Reads the given number of blocks into the buffer from the device,
    /// starting at the given offset.
    ///
    /// The caller must ensure that the `buf` buffer has enough space allocated.
    ///
    /// Operation is performed asynchronously; I/O completion status is wrapped
    /// into `CoreError::ReadFailed` in the case of failure.
    async fn read_buf_blocks_async(
        &self,
        buf: &mut DmaBuf,
        offset_blocks: u64,
        num_blocks: u64,
        opts: ReadOptions,
    ) -> Result<(), CoreError> {
        self.readv_blocks_async(&mut [buf.to_io_vec()], offset_blocks, num_blocks, opts)
            .await
    }

    /// Writes the given number of blocks from the list of buffers to the
    /// device, starting at the given offset.
    ///
    /// The caller must ensure that the number of blocks to write does not go
    /// beyond the size of `iovs` array.
    ///
    /// The given completion callback is called when the operation finishes.
    /// This method may return error immediately in the case operation dispatch
    /// fails.
    fn writev_blocks(
        &self,
        iovs: &[IoVec],
        offset_blocks: u64,
        num_blocks: u64,
        cb: IoCompletionCallback,
        cb_arg: IoCompletionCallbackArg,
    ) -> Result<(), CoreError>;

    /// Writes the given number of blocks from the list of buffers to the
    /// device, starting at the given offset.
    ///
    /// The caller must ensure that the number of blocks to write does not go
    /// beyond the size of `iovs` array.
    ///
    /// Operation is performed asynchronously; I/O completion status is wrapped
    /// into `CoreError::WriteFailed` in the case of failure.
    async fn writev_blocks_async(
        &self,
        iovs: &[IoVec],
        offset_blocks: u64,
        num_blocks: u64,
    ) -> Result<(), CoreError> {
        let (s, r) = oneshot::channel::<IoCompletionStatus>();

        self.writev_blocks(
            iovs,
            offset_blocks,
            num_blocks,
            block_device_io_completion,
            cb_arg(s),
        )?;

        match r.await.expect("Failed awaiting at writev_blocks()") {
            IoCompletionStatus::Success => Ok(()),
            status => Err(CoreError::WriteFailed {
                status,
                offset: offset_blocks,
                len: num_blocks,
            }),
        }
    }

    /// Writes the given number of blocks from the buffer to the device,
    /// starting at the given offset.
    ///
    /// The caller must ensure that the `buf` buffer is large enough to write
    /// `num_blocks`.
    ///
    /// Operation is performed asynchronously; I/O completion status is wrapped
    /// into `CoreError::WriteFailed` in the case of failure.
    async fn write_buf_blocks_async(
        &self,
        buf: &DmaBuf,
        offset_blocks: u64,
        num_blocks: u64,
    ) -> Result<(), CoreError> {
        self.writev_blocks_async(&[buf.to_io_vec()], offset_blocks, num_blocks)
            .await
    }

    /// Submits a compare request to the block device.
    ///
    /// The given completion callback is called when the operation finishes.
    /// This method may return error immediately in the case operation dispatch
    /// fails.
    ///
    /// If compare fails, the operation completes with `IoCompletionStatus` set
    /// to `NvmeError`, with
    /// `NvmeStatus::MediaError(MediaErrorStatusCode::CompareFailure))`.
    fn comparev_blocks(
        &self,
        iovs: &[IoVec],
        offset_blocks: u64,
        num_blocks: u64,
        cb: IoCompletionCallback,
        cb_arg: IoCompletionCallbackArg,
    ) -> Result<(), CoreError>;

    /// Submits a compare request to the block device.
    ///
    /// Operation is performed asynchronously; I/O completion status is wrapped
    /// into `CoreError::CompareFailed` in the case of failure.
    ///
    /// If compare fails, the operation completes with `IoCompletionStatus` set
    /// to `NvmeError`, with
    /// `NvmeStatus::MediaError(MediaErrorStatusCode::CompareFailure))`.
    async fn comparev_blocks_async(
        &self,
        iovs: &[IoVec],
        offset_blocks: u64,
        num_blocks: u64,
    ) -> Result<(), CoreError> {
        let (s, r) = oneshot::channel::<IoCompletionStatus>();

        self.comparev_blocks(
            iovs,
            offset_blocks,
            num_blocks,
            block_device_io_completion,
            cb_arg(s),
        )?;

        match r.await.expect("Failed awaiting at comparev_blocks()") {
            IoCompletionStatus::Success => Ok(()),
            status => Err(CoreError::CompareFailed {
                status,
                offset: offset_blocks,
                len: num_blocks,
            }),
        }
    }

    /// TODO
    fn reset(
        &self,
        cb: IoCompletionCallback,
        cb_arg: IoCompletionCallbackArg,
    ) -> Result<(), CoreError>;

    /// TODO
    fn unmap_blocks(
        &self,
        offset_blocks: u64,
        num_blocks: u64,
        cb: IoCompletionCallback,
        cb_arg: IoCompletionCallbackArg,
    ) -> Result<(), CoreError>;

    /// TODO
    fn write_zeroes(
        &self,
        offset_blocks: u64,
        num_blocks: u64,
        cb: IoCompletionCallback,
        cb_arg: IoCompletionCallbackArg,
    ) -> Result<(), CoreError>;

    // NVMe only.

    /// TODO
    async fn nvme_admin_custom(&self, opcode: u8) -> Result<(), CoreError>;

    /// TODO
    async fn nvme_admin(
        &self,
        nvme_cmd: &spdk_rs::libspdk::spdk_nvme_cmd,
        buffer: Option<&mut DmaBuf>,
    ) -> Result<(), CoreError>;

    /// TODO
    async fn nvme_identify_ctrlr(&self) -> Result<DmaBuf, CoreError>;

    /// TODO
    async fn create_snapshot(&self, params: SnapshotParams) -> Result<u64, CoreError>;

    /// TODO
    async fn nvme_resv_register(
        &self,
        _current_key: u64,
        _new_key: u64,
        _register_action: u8,
        _cptpl: u8,
    ) -> Result<(), CoreError> {
        Err(CoreError::NotSupported {
            source: Errno::EOPNOTSUPP,
        })
    }

    /// TODO
    async fn nvme_resv_acquire(
        &self,
        _current_key: u64,
        _preempt_key: u64,
        _acquire_action: u8,
        _resv_type: u8,
    ) -> Result<(), CoreError> {
        Err(CoreError::NotSupported {
            source: Errno::EOPNOTSUPP,
        })
    }

    /// NVMe Reservation Release
    async fn nvme_resv_release(
        &self,
        _current_key: u64,
        _resv_type: u8,
        _release_action: u8,
    ) -> Result<(), CoreError> {
        Err(CoreError::NotSupported {
            source: Errno::EOPNOTSUPP,
        })
    }

    /// TODO
    async fn nvme_resv_report(&self, _cdw11: u32, _buffer: &mut DmaBuf) -> Result<(), CoreError> {
        Err(CoreError::NotSupported {
            source: Errno::EOPNOTSUPP,
        })
    }

    /// TODO
    async fn io_passthru(
        &self,
        nvme_cmd: &spdk_rs::libspdk::spdk_nvme_cmd,
        _buffer: Option<&mut DmaBuf>,
    ) -> Result<(), CoreError> {
        Err(CoreError::NvmeIoPassthruDispatch {
            source: Errno::EOPNOTSUPP,
            opcode: nvme_cmd.opc(),
        })
    }

    /// TODO
    async fn host_id(&self) -> Result<[u8; 16], CoreError> {
        Err(CoreError::NotSupported {
            source: Errno::EOPNOTSUPP,
        })
    }
    /// Flush the io in buffer to disk, for the Local Block Device.
    fn flush_io(
        &self,
        cb: IoCompletionCallback,
        cb_arg: IoCompletionCallbackArg,
    ) -> Result<(), CoreError>;

    /// Determines if the underlying controller is failed.
    fn is_ctrlr_failed(&self) -> bool {
        false
    }
}

fn block_device_io_completion(
    _device: &dyn BlockDevice,
    status: IoCompletionStatus,
    ctx: *mut c_void,
) {
    done_cb(ctx, status);
}

/// TODO
pub trait LbaRangeController {}

#[derive(Debug, PartialEq, Copy, Clone, strum_macros::Display)]
pub enum DeviceTimeoutAction {
    /// Abort I/O operation that times out.
    Abort,
    /// Reset the  whole device in case any single command times out.
    Reset,
    /// Do not take any actions on command timeout.
    Ignore,
    /// Remove the device from the configuration
    HotRemove,
}

/// TODO
pub trait DeviceIoController {
    /// TODO
    fn get_timeout_action(&self) -> Result<DeviceTimeoutAction, CoreError>;

    /// TODO
    fn set_timeout_action(&mut self, action: DeviceTimeoutAction) -> Result<(), CoreError>;
}
