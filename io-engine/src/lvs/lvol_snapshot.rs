use std::{
    convert::TryFrom,
    ffi::{c_ushort, c_void, CString},
    mem::zeroed,
    ops::Deref,
    os::raw::c_char,
};

use async_trait::async_trait;
use futures::{channel::oneshot, future::join_all};
use nix::errno::Errno;
use strum::{EnumCount, IntoEnumIterator};

use events_api::event::EventAction;

use spdk_rs::libspdk::{
    spdk_blob, spdk_blob_reset_used_clusters_cache, spdk_lvol, spdk_xattr_descriptor,
    vbdev_lvol_create_clone_ext, vbdev_lvol_create_snapshot_ext,
};

use crate::{
    core::{
        logical_volume::LogicalVolume,
        snapshot::{CloneParams, ISnapshotDescriptor, SnapshotDescriptor, SnapshotInfo},
        Bdev, CloneXattrs, SnapshotParams, SnapshotXattrs, UntypedBdev,
    },
    eventing::Event,
    ffihelper::{cb_arg, done_cb, IntoCString},
};

use super::{BsError, Lvol, LvsError, LvsLvol};

/// Result for low-level Lvol calls.
pub type LvolResult = Result<*mut spdk_lvol, Errno>;

///  Traits gives the common snapshot/clone interface for Local/Remote Lvol.
#[async_trait::async_trait(?Send)]
pub trait LvolSnapshotOps {
    type Error;
    type SnapshotIter;
    type Lvol;

    /// Create Snapshot Common API.
    async fn create_snapshot(&self, snap_param: SnapshotParams) -> Result<Lvol, Self::Error>;

    /// Destroy snapshot.
    async fn destroy_snapshot(mut self) -> Result<(), Self::Error>;

    /// List Snapshot details based on source UUID from which snapshot is
    /// created.
    fn list_snapshot_by_source_uuid(&self) -> Vec<SnapshotDescriptor>;
    /// List Snapshot details based on source UUID from which snapshot is
    /// created.
    fn list_lvol_snapshot_by_source_uuid(&self) -> Vec<LvolSnapshotDescriptor>;

    /// List Single snapshot details based on snapshot UUID.
    fn list_snapshot_by_snapshot_uuid(&self) -> Vec<SnapshotDescriptor>;

    /// List All Snapshot.
    fn list_all_snapshots(parent_lvol: Option<&Lvol>) -> Vec<SnapshotDescriptor>;
    /// List All Lvol Snapshots.
    fn list_all_lvol_snapshots(parent_lvol: Option<&Lvol>) -> Vec<LvolSnapshotDescriptor>;

    /// Create snapshot clone.
    async fn create_clone(&self, clone_param: CloneParams) -> Result<Self::Lvol, Self::Error>;

    /// Get clone list based on snapshot_uuid.
    fn list_clones_by_snapshot_uuid(&self) -> Vec<Self::Lvol>;

    // Get a Snapshot Iterator.
    async fn snapshot_iter(self) -> Self::SnapshotIter;

    /// List All Clones.
    fn list_all_clones() -> Vec<Self::Lvol>;

    /// Prepare snapshot xattrs.
    fn prepare_snapshot_xattrs(
        &self,
        attr_descrs: &mut [spdk_xattr_descriptor; SnapshotXattrs::COUNT],
        params: SnapshotParams,
        cstrs: &mut Vec<CString>,
    ) -> Result<(), Self::Error>;

    /// create replica snapshot inner function to call spdk snapshot create
    /// function.
    /// # Safety
    /// TODO
    unsafe fn create_snapshot_inner(
        &self,
        snap_param: &SnapshotParams,
        done_cb: unsafe extern "C" fn(*mut c_void, *mut spdk_lvol, i32),
        done_cb_arg: *mut c_void,
    ) -> Result<(), Self::Error>;

    /// Supporting function for creating local snapshot.
    async fn do_create_snapshot(
        &self,
        snap_param: SnapshotParams,
        done_cb: unsafe extern "C" fn(*mut c_void, *mut spdk_lvol, i32),
        done_cb_arg: *mut c_void,
        receiver: oneshot::Receiver<LvolResult>,
    ) -> Result<Self::Lvol, Self::Error>;

    /// Prepare clone xattrs.
    fn prepare_clone_xattrs(
        &self,
        attr_descrs: &mut [spdk_xattr_descriptor; CloneXattrs::COUNT],
        params: CloneParams,
        cstrs: &mut Vec<CString>,
    ) -> Result<(), Self::Error>;

    /// Create clone inner function to call spdk clone function.
    /// # Safety
    /// TODO
    unsafe fn create_clone_inner(
        &self,
        clone_param: &CloneParams,
        done_cb: unsafe extern "C" fn(*mut c_void, *mut spdk_lvol, i32),
        done_cb_arg: *mut c_void,
    ) -> Result<(), Self::Error>;

    /// Supporting function for creating clone.
    async fn do_create_clone(
        &self,
        clone_param: CloneParams,
        done_cb: unsafe extern "C" fn(*mut c_void, *mut spdk_lvol, i32),
        done_cb_arg: *mut c_void,
        receiver: oneshot::Receiver<LvolResult>,
    ) -> Result<Self::Lvol, Self::Error>;

    /// Common API to set SnapshotDescriptor for ListReplicaSnapshot.
    fn snapshot_descriptor(&self, parent: Option<&Lvol>) -> Option<SnapshotDescriptor>;
    /// Common API to set SnapshotInfo for ListReplicaSnapshot.
    fn snapshot_descriptor_info(&self, parent: Option<&Lvol>) -> Option<SnapshotInfo>;
    /// Common API to set LvolSnapshotDescriptor for ListReplicaSnapshot.
    fn lvol_snapshot_descriptor(&self, parent: Option<&Lvol>) -> Option<LvolSnapshotDescriptor>;

    /// Return bool value to indicate, if the snapshot is marked as discarded.
    fn is_discarded_snapshot(&self) -> bool;

    /// During destroying the last linked cloned, if there is any fault
    /// happened, it is possible that, last clone can be deleted, but linked
    /// snapshot marked as discarded still present in the system. As part of
    /// pool import, do the garbage collection to clean the discarded snapshots
    /// leftout in the system.
    async fn destroy_pending_discarded_snapshot();

    /// If self is clone or a snapshot whose parent is clone, then do ancestor
    /// calculation for all snapshot linked to clone.
    fn calculate_clone_source_snap_usage(&self, total_ancestor_snap_size: u64) -> Option<u64>;

    /// Reset snapshot tree usage cache. if the lvol is replica, then reset
    /// cache will be based on replica uuid, which is parent uuid for all
    /// snapshots created from the replica. if the lvol is not replica, then
    /// reset cache  will be judge based on lvol tree present in the system.
    fn reset_snapshot_tree_usage_cache(&self, is_replica: bool);
}

/// Snapshot Descriptor to respond back as part of listsnapshot.
#[derive(Debug)]
pub struct LvolSnapshotDescriptor {
    pub snapshot_lvol: Lvol,
    pub info: SnapshotInfo,
}
impl Deref for LvolSnapshotDescriptor {
    type Target = SnapshotInfo;

    fn deref(&self) -> &Self::Target {
        self.info()
    }
}

impl From<LvolSnapshotDescriptor> for SnapshotDescriptor {
    fn from(value: LvolSnapshotDescriptor) -> Self {
        Self::new(value.snapshot_lvol, value.info)
    }
}

impl LvolSnapshotDescriptor {
    /// Create an `LvolSnapshotDescriptor` based on the snapshot `Lvol`
    /// and the `VolumeSnapshotInfo`.
    pub fn new(snapshot_lvol: Lvol, info: SnapshotInfo) -> Self {
        Self {
            snapshot_lvol,
            info,
        }
    }
    /// Get the snapshot lvol.
    pub fn snapshot_lvol(&self) -> &Lvol {
        &self.snapshot_lvol
    }
    /// Get snapshot info.
    pub fn info(&self) -> &SnapshotInfo {
        &self.info
    }
}

/// TODO
pub trait AsyncParentIterator {
    type Item;
    fn parent(&mut self) -> Option<Self::Item>;
}

/// Iterator over Lvol Blobstore for Snapshot.
pub struct LvolSnapshotIter {
    inner_blob: *mut spdk_blob,
    inner_lvol: Lvol,
}

impl LvolSnapshotIter {
    pub fn new(lvol: Lvol) -> Self {
        Self {
            inner_blob: lvol.bs_iter_first(),
            inner_lvol: lvol,
        }
    }
}

/// Iterator implementation for LvolSnapshot.
impl AsyncParentIterator for LvolSnapshotIter {
    type Item = LvolSnapshotDescriptor;
    fn parent(&mut self) -> Option<Self::Item> {
        if self.inner_blob.is_null() {
            None
        } else {
            let parent_blob = unsafe { self.inner_lvol.bs_iter_parent(self.inner_blob) }?;
            let uuid = Lvol::get_blob_xattr(parent_blob, SnapshotXattrs::SnapshotUuid.name())?;
            let snap_lvol = UntypedBdev::lookup_by_uuid_str(&uuid)
                .and_then(|bdev| Lvol::try_from(bdev).ok())?;
            self.inner_blob = parent_blob;
            self.inner_lvol = snap_lvol.clone();
            snap_lvol.lvol_snapshot_descriptor(None)
        }
    }
}

#[async_trait(?Send)]
impl LvolSnapshotOps for Lvol {
    type Error = LvsError;
    type SnapshotIter = LvolSnapshotIter;
    type Lvol = Lvol;

    /// Prepare snapshot xattrs.
    fn prepare_snapshot_xattrs(
        &self,
        attr_descrs: &mut [spdk_xattr_descriptor; SnapshotXattrs::COUNT],
        params: SnapshotParams,
        cstrs: &mut Vec<CString>,
    ) -> Result<(), LvsError> {
        for (idx, attr) in SnapshotXattrs::iter().enumerate() {
            // Get attribute value from snapshot params.
            let av = match attr {
                SnapshotXattrs::TxId => match params.txn_id() {
                    Some(v) => v,
                    None => {
                        return Err(LvsError::SnapshotConfigFailed {
                            name: self.as_bdev().name().to_string(),
                            msg: "txn id not provided".to_string(),
                        })
                    }
                },
                SnapshotXattrs::EntityId => match params.entity_id() {
                    Some(v) => v,
                    None => {
                        return Err(LvsError::SnapshotConfigFailed {
                            name: self.as_bdev().name().to_string(),
                            msg: "entity id not provided".to_string(),
                        })
                    }
                },
                SnapshotXattrs::ParentId => match params.parent_id() {
                    Some(v) => v,
                    None => {
                        return Err(LvsError::SnapshotConfigFailed {
                            name: self.as_bdev().name().to_string(),
                            msg: "parent id not provided".to_string(),
                        })
                    }
                },
                SnapshotXattrs::SnapshotUuid => match params.snapshot_uuid() {
                    Some(v) => v,
                    None => {
                        return Err(LvsError::SnapshotConfigFailed {
                            name: self.as_bdev().name().to_string(),
                            msg: "snapshot_uuid not provided".to_string(),
                        })
                    }
                },
                SnapshotXattrs::SnapshotCreateTime => match params.create_time() {
                    Some(v) => v,
                    None => {
                        return Err(LvsError::SnapshotConfigFailed {
                            name: self.as_bdev().name().to_string(),
                            msg: "create_time not provided".to_string(),
                        })
                    }
                },
                SnapshotXattrs::DiscardedSnapshot => params.discarded_snapshot().to_string(),
            };
            let attr_name = attr.name().to_string().into_cstring();
            let attr_val = av.into_cstring();
            attr_descrs[idx].name = attr_name.as_ptr() as *mut c_char;
            attr_descrs[idx].value = attr_val.as_ptr() as *mut c_void;
            attr_descrs[idx].value_len = attr_val.to_bytes().len() as c_ushort;

            cstrs.push(attr_val);
            cstrs.push(attr_name);
        }

        Ok(())
    }

    /// create replica snapshot inner function to call spdk snapshot create
    /// function.
    unsafe fn create_snapshot_inner(
        &self,
        snap_param: &SnapshotParams,
        cb: unsafe extern "C" fn(*mut c_void, *mut spdk_lvol, i32),
        cb_arg: *mut c_void,
    ) -> Result<(), LvsError> {
        let mut attr_descrs: [spdk_xattr_descriptor; SnapshotXattrs::COUNT] =
            [unsafe { zeroed() }; SnapshotXattrs::COUNT];

        // Vector to keep allocated CStrings before snapshot  creation
        // is complete to guarantee validity of attribute buffers
        // stored inside CStrings.
        let mut cstrs: Vec<CString> = Vec::new();

        self.prepare_snapshot_xattrs(&mut attr_descrs, snap_param.clone(), &mut cstrs)?;

        let c_snapshot_name = snap_param.name().unwrap().into_cstring();

        // No need to flush blob's buffers explicitly as SPDK always
        // synchronizes blob when taking a snapshot.
        unsafe {
            vbdev_lvol_create_snapshot_ext(
                self.as_inner_ptr(),
                c_snapshot_name.as_ptr(),
                attr_descrs.as_mut_ptr(),
                SnapshotXattrs::COUNT as u32,
                Some(cb),
                cb_arg,
            )
        };
        Ok(())
    }

    /// Creates a snapshot.
    async fn do_create_snapshot(
        &self,
        snap_param: SnapshotParams,
        cb: unsafe extern "C" fn(*mut c_void, *mut spdk_lvol, i32),
        cb_arg: *mut c_void,
        receiver: oneshot::Receiver<LvolResult>,
    ) -> Result<Lvol, LvsError> {
        unsafe {
            self.create_snapshot_inner(&snap_param, cb, cb_arg)?;
        }

        // Wait till operation succeeds, if requested.
        let res = receiver.await.expect("Snapshot done callback disappeared");

        match res {
            Ok(lvol_ptr) => {
                snap_param.event(EventAction::Create).generate();
                Ok(Lvol::from_inner_ptr(lvol_ptr))
            }
            Err(e) => Err(LvsError::SnapshotCreate {
                source: BsError::from_errno(e),
                msg: snap_param.name().unwrap(),
            }),
        }
    }

    /// Prepare clone xattrs.
    fn prepare_clone_xattrs(
        &self,
        attr_descrs: &mut [spdk_xattr_descriptor; CloneXattrs::COUNT],
        params: CloneParams,
        cstrs: &mut Vec<CString>,
    ) -> Result<(), LvsError> {
        for (idx, attr) in CloneXattrs::iter().enumerate() {
            // Get attribute value from CloneParams.
            let av = match attr {
                CloneXattrs::SourceUuid => match params.source_uuid() {
                    Some(v) => v,
                    None => {
                        return Err(LvsError::CloneConfigFailed {
                            name: self.as_bdev().name().to_string(),
                            msg: "source uuid not provided".to_string(),
                        })
                    }
                },
                CloneXattrs::CloneCreateTime => match params.clone_create_time() {
                    Some(v) => v,
                    None => {
                        return Err(LvsError::CloneConfigFailed {
                            name: self.as_bdev().name().to_string(),
                            msg: "create_time not provided".to_string(),
                        })
                    }
                },
                CloneXattrs::CloneUuid => match params.clone_uuid() {
                    Some(v) => v,
                    None => {
                        return Err(LvsError::CloneConfigFailed {
                            name: self.as_bdev().name().to_string(),
                            msg: "clone_uuid not provided".to_string(),
                        })
                    }
                },
            };
            let attr_name = attr.name().to_string().into_cstring();
            let attr_val = av.into_cstring();
            attr_descrs[idx].name = attr_name.as_ptr() as *mut c_char;
            attr_descrs[idx].value = attr_val.as_ptr() as *mut c_void;
            attr_descrs[idx].value_len = attr_val.to_bytes().len() as c_ushort;

            cstrs.push(attr_val);
            cstrs.push(attr_name);
        }
        Ok(())
    }

    /// Create clone inner function to call spdk clone function.
    unsafe fn create_clone_inner(
        &self,
        clone_param: &CloneParams,
        cb: unsafe extern "C" fn(*mut c_void, *mut spdk_lvol, i32),
        cb_arg: *mut c_void,
    ) -> Result<(), LvsError> {
        let mut attr_descrs: [spdk_xattr_descriptor; CloneXattrs::COUNT] =
            [unsafe { zeroed() }; CloneXattrs::COUNT];

        // Vector to keep allocated CStrings before snapshot  creation
        // is complete to guarantee validity of attribute buffers
        // stored inside CStrings.
        let mut cstrs: Vec<CString> = Vec::new();

        self.prepare_clone_xattrs(&mut attr_descrs, clone_param.clone(), &mut cstrs)?;

        let c_clone_name = clone_param.clone_name().unwrap_or_default().into_cstring();

        unsafe {
            vbdev_lvol_create_clone_ext(
                self.as_inner_ptr(),
                c_clone_name.as_ptr(),
                attr_descrs.as_mut_ptr(),
                CloneXattrs::COUNT as u32,
                Some(cb),
                cb_arg,
            )
        };
        Ok(())
    }

    /// Creates a clone.
    async fn do_create_clone(
        &self,
        clone_param: CloneParams,
        cb: unsafe extern "C" fn(*mut c_void, *mut spdk_lvol, i32),
        cb_arg: *mut c_void,
        receiver: oneshot::Receiver<LvolResult>,
    ) -> Result<Lvol, LvsError> {
        unsafe {
            self.create_clone_inner(&clone_param, cb, cb_arg)?;
        }
        // Wait till operation succeeds, if requested.
        let res = receiver
            .await
            .expect("Snapshot Clone done callback disappeared");

        match res {
            Ok(lvol_ptr) => {
                clone_param.event(EventAction::Create).generate();
                Ok(Lvol::from_inner_ptr(lvol_ptr))
            }
            Err(err) => Err(LvsError::SnapshotCloneCreate {
                source: BsError::from_errno(err),
                msg: clone_param.clone_name().unwrap_or_default(),
            }),
        }
    }

    /// Common API to set SnapshotDescriptor for ListReplicaSnapshot.
    fn snapshot_descriptor(&self, parent: Option<&Lvol>) -> Option<SnapshotDescriptor> {
        let info = self.snapshot_descriptor_info(parent)?;
        Some(SnapshotDescriptor::new(self.to_owned(), info))
    }

    /// Common API to set SnapshotDescriptor for ListReplicaSnapshot.
    fn lvol_snapshot_descriptor(&self, parent: Option<&Lvol>) -> Option<LvolSnapshotDescriptor> {
        let info = self.snapshot_descriptor_info(parent)?;
        Some(LvolSnapshotDescriptor::new(self.to_owned(), info))
    }

    /// Common API to set SnapshotDescriptor for ListReplicaSnapshot.
    fn snapshot_descriptor_info(&self, parent: Option<&Lvol>) -> Option<SnapshotInfo> {
        let mut valid_snapshot = true;
        let mut snapshot_param: SnapshotParams = Default::default();
        for attr in SnapshotXattrs::iter() {
            let curr_attr_val = match Self::get_blob_xattr(self.blob_checked(), attr.name()) {
                Some(val) => val,
                None => {
                    valid_snapshot = false;
                    continue;
                }
            };
            match attr {
                SnapshotXattrs::ParentId => {
                    if let Some(parent_lvol) = parent {
                        // Skip snapshots if it's parent is not matched.
                        if curr_attr_val != parent_lvol.uuid() {
                            return None;
                        }
                    }
                    snapshot_param.set_parent_id(curr_attr_val);
                }
                SnapshotXattrs::EntityId => {
                    snapshot_param.set_entity_id(curr_attr_val);
                }
                SnapshotXattrs::TxId => {
                    snapshot_param.set_txn_id(curr_attr_val);
                }
                SnapshotXattrs::SnapshotUuid => {
                    snapshot_param.set_snapshot_uuid(curr_attr_val);
                }
                SnapshotXattrs::SnapshotCreateTime => {
                    snapshot_param.set_create_time(curr_attr_val);
                }
                SnapshotXattrs::DiscardedSnapshot => {
                    snapshot_param
                        .set_discarded_snapshot(curr_attr_val.parse().unwrap_or_default());
                }
            }
        }
        // set remaining snapshot parameters for snapshot list
        snapshot_param.set_name(self.name());
        // set parent replica uuid and size of the snapshot
        let parent_uuid = if let Some(parent_lvol) = parent {
            parent_lvol.uuid()
        } else {
            match Bdev::lookup_by_uuid_str(snapshot_param.parent_id().unwrap_or_default().as_str())
                .and_then(|b| Lvol::try_from(b).ok())
            {
                Some(parent) => parent.uuid(),
                None => String::default(),
            }
        };
        let snapshot_descriptor = SnapshotInfo::new(
            parent_uuid,
            self.allocated(),
            snapshot_param,
            self.list_clones_by_snapshot_uuid().len() as u64,
            valid_snapshot,
        );
        Some(snapshot_descriptor)
    }

    /// Create Snapshot Common API for Local Device.
    async fn create_snapshot(&self, snap_param: SnapshotParams) -> Result<Lvol, LvsError> {
        extern "C" fn snapshot_create_done_cb(
            arg: *mut c_void,
            lvol_ptr: *mut spdk_lvol,
            errno: i32,
        ) {
            let res = if errno == 0 {
                Ok(lvol_ptr)
            } else {
                assert!(errno < 0);
                let e = Errno::from_raw(-errno);
                error!("Create snapshot failed with errno {errno}: {e}");
                Err(e)
            };

            done_cb(arg, res);
        }

        let (s, r) = oneshot::channel::<LvolResult>();

        self.do_create_snapshot(snap_param, snapshot_create_done_cb, cb_arg(s), r)
            .await
    }

    /// Get a Snapshot Iterator.
    async fn snapshot_iter(self) -> LvolSnapshotIter {
        LvolSnapshotIter::new(self)
    }

    /// Destroy snapshot.
    async fn destroy_snapshot(mut self) -> Result<(), Self::Error> {
        if self.list_clones_by_snapshot_uuid().is_empty() {
            self.destroy().await?;
        } else {
            self.set_blob_attr(
                SnapshotXattrs::DiscardedSnapshot.name(),
                true.to_string(),
                true,
            )
            .await?;
        }

        Ok(())
    }

    /// List Snapshot details based on source UUID from which snapshot is
    /// created.
    fn list_snapshot_by_source_uuid(&self) -> Vec<SnapshotDescriptor> {
        self.list_lvol_snapshot_by_source_uuid()
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>()
    }
    /// List Snapshot details based on source UUID from which snapshot is
    /// created.
    fn list_lvol_snapshot_by_source_uuid(&self) -> Vec<LvolSnapshotDescriptor> {
        let mut snapshot_list: Vec<LvolSnapshotDescriptor> = Vec::new();
        let mut lvol_snap_iter = LvolSnapshotIter::new(self.clone());
        let self_uuid = self.uuid();
        while let Some(volume_snap_descr) = lvol_snap_iter.parent() {
            // break the blob iteration if source uuid is not matched.
            // it will happen when clone snapshot list is done through
            // source clone uuid.
            if volume_snap_descr.source_uuid != self_uuid {
                break;
            }
            snapshot_list.push(volume_snap_descr);
        }
        snapshot_list
    }

    /// List Single snapshot details based on snapshot UUID.
    fn list_snapshot_by_snapshot_uuid(&self) -> Vec<SnapshotDescriptor> {
        let mut snapshot_list: Vec<SnapshotDescriptor> = Vec::new();
        if let Some(snapshot) = self.snapshot_descriptor(None) {
            snapshot_list.push(snapshot)
        }
        snapshot_list
    }

    /// List All Snapshot.
    fn list_all_snapshots(parent_lvol: Option<&Lvol>) -> Vec<SnapshotDescriptor> {
        Self::list_all_lvol_snapshots(parent_lvol)
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>()
    }

    /// List All Lvol Snapshots.
    fn list_all_lvol_snapshots(parent_lvol: Option<&Lvol>) -> Vec<LvolSnapshotDescriptor> {
        let mut snapshot_list: Vec<LvolSnapshotDescriptor> = Vec::new();

        let bdev = match UntypedBdev::bdev_first() {
            Some(b) => b,
            None => return Vec::new(), /* No devices available, provide no
                                       snapshots */
        };

        let lvol_devices = bdev
            .into_iter()
            .filter(|b| b.driver() == "lvol")
            .map(|b| Lvol::try_from(b).unwrap())
            .collect::<Vec<Lvol>>();

        for snapshot_lvol in lvol_devices {
            // skip lvol if it is not snapshot.
            if !snapshot_lvol.is_snapshot() {
                continue;
            }
            match snapshot_lvol.lvol_snapshot_descriptor(parent_lvol) {
                Some(snapshot_descriptor) => snapshot_list.push(snapshot_descriptor),
                None => continue,
            }
        }
        snapshot_list
    }

    /// Create snapshot clone.
    async fn create_clone(&self, clone_param: CloneParams) -> Result<Self::Lvol, Self::Error> {
        extern "C" fn clone_done_cb(arg: *mut c_void, lvol_ptr: *mut spdk_lvol, errno: i32) {
            let res = if errno == 0 {
                Ok(lvol_ptr)
            } else {
                assert!(errno < 0);
                let e = Errno::from_raw(-errno);
                error!("Snapshot Clone failed with errno {errno}: {e}");
                Err(e)
            };

            done_cb(arg, res);
        }

        let (s, r) = oneshot::channel::<LvolResult>();

        self.do_create_clone(clone_param, clone_done_cb, cb_arg(s), r)
            .await
    }

    /// List clones based on snapshot_uuid.
    fn list_clones_by_snapshot_uuid(&self) -> Vec<Lvol> {
        let bdev = match UntypedBdev::bdev_first() {
            Some(b) => b,
            None => return Vec::new(), /* No devices available, no clones */
        };
        bdev.into_iter()
            .filter(|b| b.driver() == "lvol")
            .map(|b| Lvol::try_from(b).unwrap())
            .filter_map(|b| {
                let snap_lvol = b.is_snapshot_clone();
                if snap_lvol.is_some() && snap_lvol.unwrap().uuid() == self.uuid() {
                    Some(b)
                } else {
                    None
                }
            })
            .collect::<Vec<Lvol>>()
    }

    /// List All Clones.
    fn list_all_clones() -> Vec<Lvol> {
        let bdev = match UntypedBdev::bdev_first() {
            Some(b) => b,
            None => return Vec::new(), /* No devices available, no clones */
        };
        bdev.into_iter()
            .filter(|b| b.driver() == "lvol")
            .map(|b| Lvol::try_from(b).unwrap())
            .filter(|b| b.is_snapshot_clone().is_some())
            .collect::<Vec<Lvol>>()
    }

    /// Check if the snapshot has been discarded.
    fn is_discarded_snapshot(&self) -> bool {
        Lvol::get_blob_xattr(
            self.blob_checked(),
            SnapshotXattrs::DiscardedSnapshot.name(),
        )
        .unwrap_or_default()
        .parse()
        .unwrap_or_default()
    }

    /// During destroying the last linked cloned, if there is any fault
    /// happened, it is possible that, last clone can be deleted, but linked
    /// snapshot marked as discarded still present in the system. As part of
    /// pool import, do the garbage collection to clean the discarded snapshots
    /// leftout in the system.
    async fn destroy_pending_discarded_snapshot() {
        let Some(bdev) = UntypedBdev::bdev_first() else {
            return; /* No devices available */
        };
        let snap_list = bdev
            .into_iter()
            .filter(|b| b.driver() == "lvol")
            .map(|b| Lvol::try_from(b).unwrap())
            .filter(|b| {
                b.is_snapshot()
                    && b.is_discarded_snapshot()
                    && b.list_clones_by_snapshot_uuid().is_empty()
            })
            .collect::<Vec<Lvol>>();
        for snap in &snap_list {
            snap.reset_snapshot_tree_usage_cache(false);
        }
        let futures = snap_list.into_iter().map(|s| s.destroy());
        let result = join_all(futures).await;
        for r in result {
            match r {
                Ok(r) => {
                    debug!("Pending discarded snapshot {r:?} destroy success")
                }
                _ => warn!("Pending discarded snapshot destroy failed"),
            }
        }
    }

    // if self is clone or a snapshot whose parent is clone, then do ancestor
    // calculation for all snapshot linked to clone.
    fn calculate_clone_source_snap_usage(&self, total_ancestor_snap_size: u64) -> Option<u64> {
        // if self is snapshot created from clone.
        if self.is_snapshot() {
            match UntypedBdev::lookup_by_uuid_str(
                &Lvol::get_blob_xattr(self.blob_checked(), SnapshotXattrs::ParentId.name())
                    .unwrap_or_default(),
            ) {
                Some(bdev) => match Lvol::try_from(bdev) {
                    Ok(l) => match l.is_snapshot_clone() {
                        Some(parent_snap_lvol) => {
                            let usage = parent_snap_lvol.usage();
                            Some(
                                total_ancestor_snap_size
                                    - (usage.allocated_bytes_snapshots + usage.allocated_bytes),
                            )
                        }
                        None => None,
                    },
                    _ => None,
                },
                _ => None,
            }
        // if self is clone.
        } else if self.is_snapshot_clone().is_some() {
            Some(
                Lvol::list_all_lvol_snapshots(Some(self))
                    .iter()
                    .map(|v| v.snapshot_lvol().allocated())
                    .sum(),
            )
        } else {
            None
        }
    }

    /// Reset snapshot tree usage cache.
    fn reset_snapshot_tree_usage_cache(&self, is_replica: bool) {
        if is_replica {
            reset_snapshot_tree_usage_cache_with_parent_uuid(self);
            return;
        }
        if let Some(snapshot_parent_uuid) =
            Lvol::get_blob_xattr(self.blob_checked(), SnapshotXattrs::ParentId.name())
        {
            if let Some(bdev) = UntypedBdev::lookup_by_uuid_str(snapshot_parent_uuid.as_str()) {
                if let Ok(parent_lvol) = Lvol::try_from(bdev) {
                    unsafe {
                        spdk_blob_reset_used_clusters_cache(parent_lvol.blob_checked());
                    }
                    reset_snapshot_tree_usage_cache_with_parent_uuid(&parent_lvol);
                }
            } else {
                reset_snapshot_tree_usage_cache_with_wildcard(self, snapshot_parent_uuid);
            }
        }
    }
}

/// When snapshot is destroyed, if snapshot parent exist, reset cache of
/// linked snapshot and clone tree based on snapshot parent.
fn reset_snapshot_tree_usage_cache_with_parent_uuid(lvol: &Lvol) {
    let mut lvol_iter = LvolSnapshotIter::new(lvol.clone());
    while let Some(volume_snap_descr) = lvol_iter.parent() {
        let curr_snap_lvol = volume_snap_descr.snapshot_lvol();
        unsafe {
            spdk_blob_reset_used_clusters_cache(curr_snap_lvol.blob_checked());
        }
        let clone_list = curr_snap_lvol.list_clones_by_snapshot_uuid();
        for clone in clone_list {
            unsafe {
                spdk_blob_reset_used_clusters_cache(clone.blob_checked());
            }
        }
    }
}

/// When snapshot is destroyed, if snapshot parent not exist, reset cache of
/// linked snapshot and clone tree based on wildcard search through complete
/// bdev by matching parent uuid got from snapshot attribute.
/// todo: need more optimization to adding new function in spdk to relate
/// snapshot and clone blobs.
fn reset_snapshot_tree_usage_cache_with_wildcard(lvol: &Lvol, snapshot_parent_uuid: String) {
    let mut successor_clones: Vec<Lvol> = vec![];

    let mut successor_snapshots = Lvol::list_all_lvol_snapshots(None)
        .iter()
        .map(|v| v.snapshot_lvol())
        .filter_map(|l| {
            let uuid = Lvol::get_blob_xattr(lvol.blob_checked(), SnapshotXattrs::ParentId.name());
            match uuid {
                Some(uuid) if uuid == snapshot_parent_uuid => Some(l.clone()),
                _ => None,
            }
        })
        .collect::<Vec<Lvol>>();

    while !successor_snapshots.is_empty() || !successor_clones.is_empty() {
        if let Some(snapshot) = successor_snapshots.pop() {
            unsafe {
                spdk_blob_reset_used_clusters_cache(snapshot.blob_checked());
            }
            let new_clone_list = snapshot.list_clones_by_snapshot_uuid();
            successor_clones.extend(new_clone_list);
        }

        if let Some(clone) = successor_clones.pop() {
            unsafe {
                spdk_blob_reset_used_clusters_cache(clone.blob_checked());
            }
            let new_snap_list = Lvol::list_all_lvol_snapshots(Some(&clone))
                .iter()
                .map(|v| v.snapshot_lvol().clone())
                .collect::<Vec<Lvol>>();
            successor_snapshots.extend(new_snap_list);
        }
    }
}
