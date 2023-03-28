use std::{
    fmt::{Debug, Formatter},
    ops::{Deref, DerefMut},
};

use libc::c_void;
use nix::errno::Errno;

use spdk_rs::{
    libspdk::{spdk_bdev_io, spdk_io_channel},
    BdevIo,
};

use super::{
    nexus_lookup_mut,
    FaultReason,
    Nexus,
    NexusChannel,
    NexusState,
    NEXUS_PRODUCT_ID,
};

use crate::core::{
    device_cmd_queue,
    BlockDevice,
    BlockDeviceHandle,
    CoreError,
    Cores,
    DeviceCommand,
    GenericStatusCode,
    IoCompletionStatus,
    IoStatus,
    IoSubmissionFailure,
    IoType,
    LvolFailure,
    Mthread,
    NvmeStatus,
    Reactors,
};

/// TODO
#[repr(C)]
#[derive(Debug)]
pub(super) struct NioCtx<'n> {
    /// number of IO's submitted. Nexus IO's may never be freed until this
    /// counter drops to zero.
    in_flight: u8,
    /// intermediate status of the IO
    status: IoStatus,
    /// a reference to  our channel
    channel: spdk_rs::IoChannel<NexusChannel<'n>>,
    /// the IO must fail regardless of when it completes
    must_fail: bool,
}

/// TODO
#[repr(transparent)]
pub(super) struct NexusBio<'n>(BdevIo<Nexus<'n>>);

impl<'n> Debug for NexusBio<'n> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} I/O [{:?}]", self.channel(), self.ctx().status)
    }
}

impl<'n> Deref for NexusBio<'n> {
    type Target = BdevIo<Nexus<'n>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for NexusBio<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'n> From<*mut spdk_bdev_io> for NexusBio<'n> {
    fn from(ptr: *mut spdk_bdev_io) -> Self {
        Self(BdevIo::<Nexus<'n>>::legacy_from_ptr(ptr))
    }
}

impl Clone for NexusBio<'_> {
    fn clone(&self) -> Self {
        Self::new(self.ctx().channel.clone(), self.0.clone())
    }
}

impl<'n> NexusBio<'n> {
    fn as_ptr(&self) -> *mut spdk_bdev_io {
        self.0.legacy_as_ptr()
    }

    /// Makes a new instance of `NexusBio` from a channel and `BdevIo`.
    pub(super) fn new(
        channel: spdk_rs::IoChannel<NexusChannel<'n>>,
        io: BdevIo<Nexus<'n>>,
    ) -> Self {
        let mut bio = NexusBio(io);
        let ctx = bio.ctx_mut();
        ctx.channel = channel;
        ctx.status = IoStatus::Pending;
        ctx.in_flight = 0;
        ctx.must_fail = false;
        bio
    }

    /// TODO
    pub(super) fn submit_request(mut self) {
        if let Err(_e) = match self.io_type() {
            IoType::Read => self.readv(),
            // these IOs are submitted to all the underlying children
            IoType::Write
            | IoType::WriteZeros
            | IoType::Reset
            | IoType::Unmap => self.submit_all(),
            IoType::Flush => {
                self.ok();
                Ok(())
            }
            IoType::NvmeAdmin => {
                self.fail();
                Err(CoreError::NotSupported {
                    source: Errno::EINVAL,
                })
            }
            _ => {
                trace!(?self, "not supported");
                self.fail();
                Err(CoreError::NotSupported {
                    source: Errno::EOPNOTSUPP,
                })
            }
        } {
            //trace!(?e, ?io, "Error during IO submission");
        }
    }

    /// assess the IO if we need to mark it failed or ok.
    /// obtain the Nexus struct embedded within the bdev
    pub(crate) fn nexus(&self) -> &Nexus<'n> {
        self.bdev_checked(NEXUS_PRODUCT_ID).data()
    }

    /// invoked when a nexus IO completes
    fn child_completion(
        device: &dyn BlockDevice,
        status: IoCompletionStatus,
        ctx: *mut c_void,
    ) {
        let mut nexus_io = NexusBio::from(ctx as *mut spdk_bdev_io);
        nexus_io.complete(device, status);
    }

    /// immutable reference to the IO context
    #[inline(always)]
    fn ctx(&self) -> &NioCtx<'n> {
        self.driver_ctx::<NioCtx>()
    }

    /// a mutable reference to the IO context
    #[inline(always)]
    fn ctx_mut(&mut self) -> &mut NioCtx<'n> {
        self.driver_ctx_mut::<NioCtx>()
    }

    /// completion handler for the nexus when a child IO completes
    fn complete(
        &mut self,
        child: &dyn BlockDevice,
        status: IoCompletionStatus,
    ) {
        let success = status == IoCompletionStatus::Success;

        debug_assert!(self.ctx().in_flight > 0);
        self.ctx_mut().in_flight -= 1;

        if success {
            self.ok_checked();
        } else {
            // IO failure, mark the IO failed and take the child out
            error!(
                "{:?}: IO completion for '{}' failed: {:?}, ctx={:?}",
                self,
                child.device_name(),
                status,
                self.ctx()
            );
            self.ctx_mut().status = IoStatus::Failed;
            self.ctx_mut().must_fail = true;
            self.handle_failure(child, status);
        }
    }

    /// Complete the IO marking at as successfully when all child IO's have been
    /// accounted for. Failing to account for all child IO's will result in
    /// a lockup.
    #[inline]
    fn ok_checked(&mut self) {
        if self.ctx().in_flight == 0 {
            if self.ctx().must_fail {
                //warn!(?self, "resubmitted due to must_fail");
                self.retry_checked();
                //self.fail();
            } else {
                self.ok();
            }
        }
    }

    /// Complete the IO marking it as failed.
    #[inline]
    fn fail_checked(&mut self) {
        if self.ctx().in_flight == 0 {
            self.fail();
        }
    }

    /// retry this IO when all other IOs have completed
    #[inline]
    fn retry_checked(&mut self) {
        if self.ctx().in_flight == 0 {
            debug!(?self, "resubmitting IO");
            self.clone().submit_request();
        }
    }

    /// reference to the channel. The channel contains the specific
    /// per-core data structures.
    #[inline(always)]
    fn channel(&self) -> &NexusChannel<'n> {
        self.ctx().channel.channel_data()
    }

    /// mutable reference to the channels. The channel contains the
    /// specific per-core data structures.
    #[inline(always)]
    fn channel_mut(&mut self) -> &mut NexusChannel<'n> {
        self.ctx_mut().channel.channel_data_mut()
    }

    /// Returns the offset in num blocks where the data partition starts.
    #[inline]
    fn data_ent_offset(&self) -> u64 {
        self.nexus().data_ent_offset
    }

    /// submit a read operation to one of the children of this nexus
    #[inline]
    fn submit_read(
        &self,
        hdl: &dyn BlockDeviceHandle,
    ) -> Result<(), CoreError> {
        #[cfg(feature = "fault_injection")]
        if self.nexus().inject_is_faulted(hdl.get_device()) {
            return Err(CoreError::ReadDispatch {
                source: Errno::ENODEV,
                offset: self.offset(),
                len: self.num_blocks(),
            });
        }

        hdl.readv_blocks(
            self.iovs(),
            self.iov_count(),
            self.offset() + self.data_ent_offset(),
            self.num_blocks(),
            Self::child_completion,
            self.as_ptr().cast(),
        )
    }

    /// Submit a Read operation to the next available replica.
    fn __do_readv_one(&mut self) -> Result<(), CoreError> {
        if let Some(hdl) = self.channel().select_reader() {
            let r = self.submit_read(hdl);

            if r.is_err() {
                // Such a situation can happen when there is no active I/O in
                // the queues, but error on qpair is observed
                // due to network timeout, which initiates
                // controller reset. During controller reset all
                // I/O channels are de-initialized, so no I/O
                // submission is possible (spdk returns -6/ENXIO), so we have to
                // start device retire.
                // TODO: ENOMEM and ENXIO should be handled differently and
                // device should not be retired in case of ENOMEM.
                let device = hdl.get_device().device_name();
                error!(
                    "{:?}: read I/O to '{}' submission failed: {:?}",
                    self, device, r
                );

                self.fault_device(
                    &device,
                    IoCompletionStatus::IoSubmissionError(
                        IoSubmissionFailure::Write,
                    ),
                );
                r
            } else {
                self.ctx_mut().in_flight = 1;
                r
            }
        } else {
            error!(
                "{:?}: read I/O submission failed: no children available",
                self
            );

            Err(CoreError::NoDevicesAvailable {})
        }
    }

    /// Submit a read operation to the next suitable replica.
    /// In case of submission error the requiest is transparently resubmitted
    /// to the next available replica.
    fn do_readv(&mut self) -> Result<(), CoreError> {
        match self.__do_readv_one() {
            Err(e) => {
                match e {
                    // No readers available - bail out.
                    CoreError::NoDevicesAvailable {} => {
                        self.fail();
                        Err(e)
                    }
                    // Failed to submit Read I/O request to the current replica,
                    // try to resumbit request to the next available replica.
                    _ => {
                        let mut num_readers = self.channel().num_readers();

                        let r = {
                            if num_readers <= 1 {
                                // No more readers available (taking into
                                // account the failed one).
                                Err(e)
                            } else {
                                num_readers -= 1; // Account the failed reader.

                                // Resubmission loop to find a next available
                                // replica for this Read I/O operation.
                                loop {
                                    match self.__do_readv_one() {
                                        Ok(_) => break Ok(()),
                                        Err(e) => {
                                            num_readers -= 1;

                                            if num_readers == 0 {
                                                break Err(e);
                                            }
                                        }
                                    }
                                }
                            }
                        };

                        if r.is_err() {
                            self.fail();
                        }
                        r
                    }
                }
            }
            Ok(_) => Ok(()),
        }
    }

    extern "C" fn nexus_get_buf_cb(
        _ch: *mut spdk_io_channel,
        io: *mut spdk_bdev_io,
        success: bool,
    ) {
        let mut bio = NexusBio::from(io);

        if !success {
            trace!(
                "(core: {} thread: {}): get_buf() failed",
                Cores::current(),
                Mthread::current().unwrap().name()
            );
            bio.no_mem();
        }

        let _ = bio.do_readv();
    }

    /// submit read IO to some child
    fn readv(&mut self) -> Result<(), CoreError> {
        if self.need_buf() {
            unsafe {
                self.alloc_buffer(Self::nexus_get_buf_cb);
            }
            Ok(())
        } else {
            self.do_readv()
        }
    }

    #[inline]
    fn submit_write(
        &self,
        hdl: &dyn BlockDeviceHandle,
    ) -> Result<(), CoreError> {
        hdl.writev_blocks(
            self.iovs(),
            self.iov_count(),
            self.offset() + self.data_ent_offset(),
            self.num_blocks(),
            Self::child_completion,
            self.as_ptr().cast(),
        )
    }

    #[inline]
    fn submit_unmap(
        &self,
        hdl: &dyn BlockDeviceHandle,
    ) -> Result<(), CoreError> {
        hdl.unmap_blocks(
            self.offset() + self.data_ent_offset(),
            self.num_blocks(),
            Self::child_completion,
            self.as_ptr().cast(),
        )
    }

    #[inline]
    fn submit_write_zeroes(
        &self,
        hdl: &dyn BlockDeviceHandle,
    ) -> Result<(), CoreError> {
        hdl.write_zeroes(
            self.offset() + self.data_ent_offset(),
            self.num_blocks(),
            Self::child_completion,
            self.as_ptr().cast(),
        )
    }

    #[inline]
    fn submit_reset(
        &self,
        hdl: &dyn BlockDeviceHandle,
    ) -> Result<(), CoreError> {
        hdl.reset(Self::child_completion, self.as_ptr().cast())
    }

    /// Submit the IO to all underlying children, failing on the first error we
    /// find. When an IO is partially submitted -- we must wait until all
    /// the child IOs have completed before we mark the whole IO failed to
    /// avoid double frees. This function handles IO for a subset that must
    /// be submitted to all the underlying children.
    fn submit_all(&mut self) -> Result<(), CoreError> {
        let mut inflight = 0;
        // Name of the device which experiences I/O submission failures.
        let mut failed_device = None;

        let result = self.channel().for_each_writer(|h| {
            match self.io_type() {
                IoType::Write => self.submit_write(h),
                IoType::Unmap => self.submit_unmap(h),
                IoType::WriteZeros => self.submit_write_zeroes(h),
                IoType::Reset => self.submit_reset(h),
                // we should never reach here, if we do it is a bug.
                _ => unreachable!(),
            }
                .map(|_| {
                    inflight += 1;
                })
                .map_err(|err| {
                    error!(
                        "(core: {} thread: {}): IO submission failed with error {:?}, I/Os submitted: {}",
                        Cores::current(), Mthread::current().unwrap().name(), err, inflight
                    );

                    // Record the name of the device for immediate retire.
                    failed_device = Some(h.get_device().device_name());
                    err
                })
        });

        // Submission errors can also trigger device retire.
        // Such a situation can happen when there is no active I/O in the
        // queues, but error on qpair is observed due to network
        // timeout, which initiates controller reset. During controller
        // reset all I/O channels are de-initialized, so no I/O
        // submission is possible (spdk returns -6/ENXIO), so we have to
        // start device retire.
        // TODO: ENOMEM and ENXIO should be handled differently and
        // device should not be retired in case of ENOMEM.
        if result.is_err() {
            let device = failed_device.unwrap();
            // set the IO as failed in the submission stage.
            self.ctx_mut().must_fail = true;

            self.channel_mut().disconnect_device(&device);

            self.fault_device(
                &device,
                IoCompletionStatus::IoSubmissionError(
                    IoSubmissionFailure::Read,
                ),
            );
        }

        // partial submission
        if inflight != 0 {
            // An error was experienced during submission. Some IO however, has
            // been submitted successfully prior to the error condition.
            self.ctx_mut().in_flight = inflight;
            self.ctx_mut().status = IoStatus::Success;
            self.ok_checked();
            return result;
        }

        self.fail_checked();

        result
    }

    /// Initiate shutdown of the nexus associated with this BIO request.
    fn try_self_shutdown_nexus(&mut self) {
        if self
            .channel_mut()
            .nexus_mut()
            .shutdown_requested
            .compare_exchange(false, true)
            .is_ok()
        {
            let nexus_name =
                self.channel_mut().nexus_mut().nexus_name().to_owned();

            Reactors::master().send_future(async move {
                if let Some(mut nexus) = nexus_lookup_mut(&nexus_name) {
                    // Check against concurrent graceful nexus shutdown
                    // initiated by user and mark nexus as being shutdown.
                    {
                        let mut s = nexus.state.lock();
                        match *s {
                            NexusState::Shutdown |
                            NexusState::ShuttingDown => {
                                info!(
                                    nexus_name,
                                    "Nexus is under user-triggered shutdown, skipping self shutdown"
                                );
                                return;
                            },
                            nexus_state => {
                                info!(
                                    nexus_name,
                                    nexus_state=%nexus_state,
                                    "Initiating self shutdown for nexus"
                                );
                            }
                        };
                        *s = NexusState::ShuttingDown;
                    }

                    // 1: Close I/O channels for all children.
                    let devices = unsafe {
                        nexus
                            .as_mut()
                            .children_iter_mut()
                            .filter_map(|c| c.get_device_name())
                            .collect::<Vec<_>>()
                    };

                    for d in devices {
                        if let Err(e) =
                            nexus.disconnect_device_from_channels(d.clone()).await
                        {
                            error!(
                                "{}: failed to disconnect I/O channels: {:?}",
                                d, e
                            );
                        }

                        device_cmd_queue().enqueue(
                            DeviceCommand::RetireDevice {
                                nexus_name: nexus.name.clone(),
                                child_device: d.clone(),
                            },
                        );
                    }

                    // Step 2: cancel all active rebuild jobs.
                    let child_uris = nexus.children_uris();
                    for child in child_uris {
                        nexus.as_mut().cancel_rebuild_jobs(&child).await;
                    }

                    // Step 3: close all children.
                    nexus.as_mut().close_children().await;

                    // Step 4: Mark nexus as shutdown.
                    // Note: we don't persist nexus's state in ETCd as nexus
                    // might be recreated on onother node.
                    *nexus.state.lock() = NexusState::Shutdown;
                }
            });
        }
    }

    /// Faults the device by its name, with the given I/O error.
    /// The faulted device is scheduled to be retired.
    fn fault_device(
        &mut self,
        child_device: &str,
        io_status: IoCompletionStatus,
    ) {
        let reason = match io_status {
            IoCompletionStatus::LvolError(LvolFailure::NoSpace) => {
                FaultReason::NoSpace
            }
            _ => FaultReason::IoError,
        };

        self.channel_mut().fault_device(child_device, reason);
    }

    /// Test handle_failure()
    fn handle_failure(
        &mut self,
        child: &dyn BlockDevice,
        status: IoCompletionStatus,
    ) {
        // We have experienced a failure on one of the child devices. We need to
        // ensure we do not submit more IOs to this child. We do not
        // need to tell other cores about this because
        // they will experience the same errors on their own channels, and
        // handle it on their own.
        //
        // We differentiate between errors in the submission and completion.
        // When we have a completion error, it typically means that the
        // child has lost the connection to the nexus. In order for
        // outstanding IO to complete, the IO's to that child must be aborted.
        // The abortion is implicit when removing the device.

        if matches!(
            status,
            IoCompletionStatus::NvmeError(NvmeStatus::Generic(
                GenericStatusCode::InvalidOpcode
            ))
        ) {
            debug!(
                "Device '{}' experienced invalid opcode error: \
                retiring skipped",
                child.device_name()
            );
            return;
        }

        // Reservation conflicts should trigger shutdown of the nexus but
        // replica should not be retired.
        if matches!(
            status,
            IoCompletionStatus::NvmeError(NvmeStatus::Generic(
                GenericStatusCode::ReservationConflict
            ))
        ) {
            warn!(
                nexus = self.channel_mut().nexus_mut().nexus_name(),
                replica=child.device_name(),
                "Reservation conflict on replica device, initiating nexus shutdown"
            );
            self.try_self_shutdown_nexus();
        } else {
            self.fault_device(&child.device_name(), status);

            let retry = matches!(
                status,
                IoCompletionStatus::NvmeError(NvmeStatus::Generic(
                    GenericStatusCode::AbortedSubmissionQueueDeleted
                ))
            );

            // if the IO was failed because of retire, resubmit the IO
            if retry {
                return self.ok_checked();
            }
        }

        self.fail_checked();
    }
}
