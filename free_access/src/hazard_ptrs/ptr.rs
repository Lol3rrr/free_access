use std::sync::atomic;

pub struct HazardPtr<T> {
    ptr: atomic::AtomicPtr<T>,
    pub next: atomic::AtomicPtr<Self>,
}

impl<T> HazardPtr<T> {
    pub fn new(data: *mut T) -> Self {
        Self {
            ptr: atomic::AtomicPtr::new(data),
            next: atomic::AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    /// This attempts to load the Protected-Ptr stored in this Hazard-Ptr.
    ///
    /// # Returns
    /// * Some(ptr): The Ptr stored in it, not Null
    /// * None: The Hazard-Ptr is currently empty and does not protect anything
    pub fn ptr(&self) -> Option<*mut T> {
        let ptr = self.ptr.load(atomic::Ordering::Acquire);
        if ptr.is_null() {
            None
        } else {
            Some(ptr)
        }
    }

    /// Attempts to store the `data` in an empty Hazard-Ptr, if the Hazard-Ptr
    /// already contains a valid Ptr (Non-Null), then this will fail and return
    /// the given `data`-Ptr
    pub fn store(&self, data: *mut T) -> Result<(), *mut T> {
        match self.ptr.compare_exchange(
            std::ptr::null_mut(),
            data,
            atomic::Ordering::SeqCst,
            atomic::Ordering::SeqCst,
        ) {
            Ok(_) => Ok(()),
            Err(_) => Err(data),
        }
    }

    /// Resets the Ptr stored in the Hazard-Ptr
    pub fn reset(&self) {
        self.ptr
            .store(std::ptr::null_mut(), atomic::Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new() {
        let ptr: HazardPtr<usize> = HazardPtr::new(std::ptr::null_mut());
        drop(ptr);
    }

    #[test]
    fn new_store() {
        let ptr: HazardPtr<usize> = HazardPtr::new(std::ptr::null_mut());
        assert_eq!(Ok(()), ptr.store(0x12 as *mut usize));

        assert_eq!(0x12 as *mut usize, ptr.ptr.load(atomic::Ordering::SeqCst));

        assert_eq!(Err(0x23 as *mut usize), ptr.store(0x23 as *mut usize));
        assert_eq!(0x12 as *mut usize, ptr.ptr.load(atomic::Ordering::SeqCst));
    }

    #[test]
    fn store_reset_store() {
        let ptr: HazardPtr<usize> = HazardPtr::new(std::ptr::null_mut());
        assert_eq!(Ok(()), ptr.store(0x12 as *mut usize));

        assert_eq!(0x12 as *mut usize, ptr.ptr.load(atomic::Ordering::SeqCst));

        ptr.reset();
        assert_eq!(0 as *mut usize, ptr.ptr.load(atomic::Ordering::SeqCst));

        assert_eq!(Ok(()), ptr.store(0x23 as *mut usize));
        assert_eq!(0x23 as *mut usize, ptr.ptr.load(atomic::Ordering::SeqCst));
    }

    #[test]
    fn ptr() {
        let ptr: HazardPtr<usize> = HazardPtr::new(std::ptr::null_mut());
        assert_eq!(Ok(()), ptr.store(0x12 as *mut usize));

        assert_eq!(0x12 as *mut usize, ptr.ptr.load(atomic::Ordering::SeqCst));
        assert_eq!(Some(0x12 as *mut usize), ptr.ptr());

        ptr.reset();
        assert_eq!(0 as *mut usize, ptr.ptr.load(atomic::Ordering::SeqCst));
        assert_eq!(None, ptr.ptr());
    }
}
