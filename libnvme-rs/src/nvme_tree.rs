/// RAII Wrapper for nvme_root_t
pub(crate) struct NvmeRoot {
    root: *mut crate::bindings::nvme_root,
}

impl NvmeRoot {
    pub(crate) fn new(root: *mut crate::bindings::nvme_root) -> Self {
        NvmeRoot { root }
    }
    pub(crate) fn as_mut_ptr(&self) -> *mut crate::bindings::nvme_root {
        self.root
    }
}

impl Drop for NvmeRoot {
    fn drop(&mut self) {
        unsafe { crate::nvme_free_tree(self.root) }
    }
}

/// Iterator for nvme_host_t
pub(crate) struct NvmeHostIterator<'a> {
    root: &'a NvmeRoot,
    host: *mut crate::bindings::nvme_host,
}

impl<'a> NvmeHostIterator<'a> {
    pub(crate) fn new(root: &'a NvmeRoot) -> Self {
        Self {
            root,
            host: std::ptr::null_mut(),
        }
    }
}

impl Iterator for NvmeHostIterator<'_> {
    type Item = *mut crate::bindings::nvme_host;

    fn next(&mut self) -> Option<Self::Item> {
        self.host = if self.host.is_null() {
            unsafe { crate::nvme_first_host(self.root.as_mut_ptr()) }
        } else {
            unsafe { crate::nvme_next_host(self.root.as_mut_ptr(), self.host) }
        };
        if self.host.is_null() {
            None
        } else {
            Some(self.host)
        }
    }
}

/// Iterator for nvme_subsystem_t
pub(crate) struct NvmeSubsystemIterator {
    host: *mut crate::bindings::nvme_host,
    subsys: *mut crate::bindings::nvme_subsystem,
}

impl NvmeSubsystemIterator {
    pub(crate) fn new(host: *mut crate::bindings::nvme_host) -> Self {
        Self {
            host,
            subsys: std::ptr::null_mut(),
        }
    }
}

impl Iterator for NvmeSubsystemIterator {
    type Item = *mut crate::bindings::nvme_subsystem;

    fn next(&mut self) -> Option<Self::Item> {
        self.subsys = if self.subsys.is_null() {
            unsafe { crate::nvme_first_subsystem(self.host) }
        } else {
            unsafe { crate::nvme_next_subsystem(self.host, self.subsys) }
        };
        if self.subsys.is_null() {
            None
        } else {
            Some(self.subsys)
        }
    }
}

/// Iterator for nvme_ctrl_t
pub(crate) struct NvmeCtrlrIterator {
    subsys: *mut crate::bindings::nvme_subsystem,
    ctrlr: *mut crate::bindings::nvme_ctrl,
}

impl NvmeCtrlrIterator {
    pub(crate) fn new(subsys: *mut crate::bindings::nvme_subsystem) -> Self {
        Self {
            subsys,
            ctrlr: std::ptr::null_mut(),
        }
    }
}

impl Iterator for NvmeCtrlrIterator {
    type Item = *mut crate::bindings::nvme_ctrl;

    fn next(&mut self) -> Option<Self::Item> {
        self.ctrlr = if self.ctrlr.is_null() {
            unsafe { crate::nvme_subsystem_first_ctrl(self.subsys) }
        } else {
            unsafe { crate::nvme_subsystem_next_ctrl(self.subsys, self.ctrlr) }
        };
        if self.ctrlr.is_null() {
            None
        } else {
            Some(self.ctrlr)
        }
    }
}

/// Iterator for nvme_ns_t given nvme_subsystem
pub(crate) struct NvmeNamespaceIterator {
    subsys: *mut crate::bindings::nvme_subsystem,
    ns: *mut crate::bindings::nvme_ns,
}

impl NvmeNamespaceIterator {
    pub(crate) fn new(subsys: *mut crate::bindings::nvme_subsystem) -> Self {
        Self {
            subsys,
            ns: std::ptr::null_mut(),
        }
    }
}

impl Iterator for NvmeNamespaceIterator {
    type Item = *mut crate::bindings::nvme_ns;

    fn next(&mut self) -> Option<Self::Item> {
        self.ns = if self.ns.is_null() {
            unsafe { crate::nvme_subsystem_first_ns(self.subsys) }
        } else {
            unsafe { crate::nvme_subsystem_next_ns(self.subsys, self.ns) }
        };
        if self.ns.is_null() {
            None
        } else {
            Some(self.ns)
        }
    }
}

/// Iterator for nvme_ns_t given nvme_ctrl
pub(crate) struct NvmeNamespaceInCtrlrIterator {
    ctrlr: *mut crate::bindings::nvme_ctrl,
    ns: *mut crate::bindings::nvme_ns,
}

impl NvmeNamespaceInCtrlrIterator {
    pub(crate) fn new(ctrlr: *mut crate::bindings::nvme_ctrl) -> Self {
        Self {
            ctrlr,
            ns: std::ptr::null_mut(),
        }
    }
}

impl Iterator for NvmeNamespaceInCtrlrIterator {
    type Item = *mut crate::bindings::nvme_ns;

    fn next(&mut self) -> Option<Self::Item> {
        self.ns = if self.ns.is_null() {
            unsafe { crate::nvme_ctrl_first_ns(self.ctrlr) }
        } else {
            unsafe { crate::nvme_ctrl_next_ns(self.ctrlr, self.ns) }
        };
        if self.ns.is_null() {
            None
        } else {
            Some(self.ns)
        }
    }
}
