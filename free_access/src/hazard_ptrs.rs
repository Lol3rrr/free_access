use std::sync::atomic;

struct HazardPtr<T> {
    used: atomic::AtomicBool,
    ptr: atomic::AtomicPtr<T>,
    next: atomic::AtomicPtr<Self>,
}

impl<T> HazardPtr<T> {
    pub fn new() -> Self {
        Self {
            used: atomic::AtomicBool::new(false),
            ptr: atomic::AtomicPtr::new(std::ptr::null_mut()),
            next: atomic::AtomicPtr::new(std::ptr::null_mut()),
        }
    }
    pub fn new_with_initial(ptr: *mut T) -> Self {
        Self {
            used: atomic::AtomicBool::new(true),
            ptr: atomic::AtomicPtr::new(ptr),
            next: atomic::AtomicPtr::new(std::ptr::null_mut()),
        }
    }
}

pub struct HazardPtrFrame<T> {
    ptrs: *mut HazardPtr<T>,
}

unsafe impl<T> Send for HazardPtrFrame<T> {}
unsafe impl<T> Sync for HazardPtrFrame<T> {}

impl<T> HazardPtrFrame<T> {
    pub fn new() -> Self {
        let initial = Box::into_raw(Box::new(HazardPtr::new()));
        Self { ptrs: initial }
    }

    pub fn store(&self, ptr: *mut T) {
        let mut current = unsafe { &*self.ptrs };

        loop {
            if !current.used.load(atomic::Ordering::Relaxed) {
                match current.used.compare_exchange(
                    false,
                    true,
                    atomic::Ordering::SeqCst,
                    atomic::Ordering::SeqCst,
                ) {
                    Ok(_) => {
                        current.ptr.store(ptr, atomic::Ordering::Release);
                        return;
                    }
                    Err(_) => {}
                };
            }

            let next = current.next.load(atomic::Ordering::Acquire);
            if next.is_null() {
                break;
            }

            current = unsafe { &*next };
        }

        let new_hazard = Box::new(HazardPtr::new_with_initial(ptr));
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

    pub fn roots(&self) -> Vec<*mut T> {
        let mut result = Vec::new();

        let mut current = unsafe { &*self.ptrs };
        loop {
            if current.used.load(atomic::Ordering::Acquire) {
                let protected = current.ptr.load(atomic::Ordering::Acquire);
                result.push(protected);
            }

            let next = current.next.load(atomic::Ordering::Acquire);
            if next.is_null() {
                break;
            }

            current = unsafe { &*next };
        }

        result
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

        let expected = vec![123 as *mut u8];
        let result = frame.roots();
        assert_eq!(expected, result);
    }
}
