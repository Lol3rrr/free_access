use std::sync::atomic;

mod ptr;
pub use ptr::HazardPtr;

pub struct HazardPtrFrame<T> {
    ptrs: *mut HazardPtr<T>,
}

unsafe impl<T> Send for HazardPtrFrame<T> {}
unsafe impl<T> Sync for HazardPtrFrame<T> {}

impl<T> HazardPtrFrame<T> {
    pub fn new() -> Self {
        let initial = Box::into_raw(Box::new(HazardPtr::new(std::ptr::null_mut())));
        Self { ptrs: initial }
    }

    /// Stores the given `ptr` in the Hazard-Ptr-Frame, by either reusing an
    /// existing empty Hazard-Ptr or creating a new Hazard-Ptr and adding it to
    /// the Hazard-Ptr-Frame
    pub fn store(&self, ptr: *mut T) {
        let mut latest_ptr = self.ptrs;
        for current_ptr in self.iter() {
            let current = unsafe { &*current_ptr };

            match current.store(ptr) {
                Ok(_) => return,
                Err(_) => {}
            };
            latest_ptr = current_ptr;
        }

        let mut current = unsafe { &*latest_ptr };

        let new_hazard = Box::new(HazardPtr::new(ptr));
        let new_hazard_ptr = Box::into_raw(new_hazard);
        loop {
            match current.next.compare_exchange(
                std::ptr::null_mut(),
                new_hazard_ptr,
                atomic::Ordering::SeqCst,
                atomic::Ordering::SeqCst,
            ) {
                Ok(_) => {
                    break;
                }
                Err(next) => {
                    current = unsafe { &*next };
                }
            };
        }
    }

    /// Clears the entire Hazard-Ptr-Frame
    pub fn clear(&self) {
        for current_ptr in self.iter() {
            let current = unsafe { &*current_ptr };

            current.reset();
        }
    }

    /// Gathers all the Ptrs stored in the Hazard-Ptr-Frame
    pub fn roots(&self) -> Vec<*mut T> {
        let mut result = Vec::new();

        for current_ptr in self.iter() {
            let current = unsafe { &*current_ptr };

            match current.ptr() {
                Some(ptr) => result.push(ptr),
                None => {}
            };
        }

        result
    }

    /// Creates an Iterator over all the Hazard-Ptr's contained in the
    /// Hazard-Ptr-Frame
    fn iter(&self) -> HazardPtrIter<T> {
        HazardPtrIter { current: self.ptrs }
    }
}

/// An Iterator over all the Hazard-Ptr's in a Hazard-Ptr-Frame
struct HazardPtrIter<T> {
    current: *mut HazardPtr<T>,
}

impl<T> Iterator for HazardPtrIter<T> {
    type Item = *mut HazardPtr<T>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_null() {
            return None;
        }

        let ptr = self.current;
        let current = unsafe { &*self.current };
        self.current = current.next.load(atomic::Ordering::Acquire);

        Some(ptr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new() {
        let frame = HazardPtrFrame::<u8>::new();
        drop(frame);
    }

    #[test]
    fn store() {
        let frame = HazardPtrFrame::new();

        frame.store(123 as *mut u8);
    }
    #[test]
    fn store_multiple() {
        let frame = HazardPtrFrame::new();

        frame.store(123 as *mut u8);
        frame.store(234 as *mut u8);
    }

    #[test]
    fn store_roots() {
        let frame = HazardPtrFrame::new();

        frame.store(123 as *mut u8);
        frame.store(234 as *mut u8);

        let expected = vec![123 as *mut u8, 234 as *mut u8];
        let result = frame.roots();
        assert_eq!(expected, result);
    }

    #[test]
    fn clear() {
        let frame = HazardPtrFrame::new();

        frame.store(123 as *mut u8);
        frame.store(234 as *mut u8);

        let expected = vec![123 as *mut u8, 234 as *mut u8];
        let result = frame.roots();
        assert_eq!(expected, result);

        frame.clear();
        let expected: Vec<*mut u8> = vec![];
        let result = frame.roots();
        assert_eq!(expected, result);
    }
}
