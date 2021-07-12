use std::{cell::UnsafeCell, sync::atomic};

mod pool;

pub struct GlobalAllocPool<T> {
    pool: pool::Pool<AllocationBuffer<T>>,
}

impl<T> GlobalAllocPool<T> {
    pub fn new() -> Self {
        Self {
            pool: pool::Pool::new(),
        }
    }

    pub fn pop(&self, phase: u64) -> Result<AllocationBuffer<T>, pool::PopError> {
        self.pool.pop(phase)
    }
    pub fn insert(&self, phase: u64, data: AllocationBuffer<T>) -> Result<(), ()> {
        self.pool.insert(data, phase)
    }

    pub fn clear(&self, n_phase: u64) -> Result<(), ()> {
        self.pool.update_phase(n_phase)?;

        Ok(())
    }
}

impl<T> Default for GlobalAllocPool<T> {
    fn default() -> Self {
        Self {
            pool: pool::Pool::new(),
        }
    }
}

unsafe impl<T> Send for GlobalAllocPool<T> {}

pub struct LocalAllocator<T> {
    buffer: UnsafeCell<AllocationBuffer<T>>,
}

impl<T> LocalAllocator<T> {
    pub fn new() -> Self {
        Self {
            buffer: UnsafeCell::new(AllocationBuffer::new()),
        }
    }

    pub fn is_empty(&self) -> bool {
        let buffer = unsafe { &*self.buffer.get() };
        buffer.is_empty()
    }

    pub fn pop(&self) -> Option<*mut T> {
        let buffer = unsafe { &*self.buffer.get() };
        buffer.pop()
    }

    pub fn insert(&self, data: *mut T) -> Result<(), *mut T> {
        let buffer = unsafe { &*self.buffer.get() };
        buffer.insert(data)
    }

    pub fn take(&self) -> AllocationBuffer<T> {
        let ptr = self.buffer.get();
        unsafe { std::ptr::replace(ptr, AllocationBuffer::new()) }
    }

    pub fn new_buffer(&self, n_buffer: AllocationBuffer<T>) {
        let ptr = self.buffer.get();
        unsafe { std::ptr::replace(ptr, n_buffer) };
    }
}

unsafe impl<T> Sync for LocalAllocator<T> {}

mod page;
pub use page::*;

const BUFFER_SIZE: usize = 128;

pub struct AllocationBuffer<T> {
    buffer: Vec<atomic::AtomicPtr<T>>,
    head: atomic::AtomicUsize,
}

impl<T> AllocationBuffer<T> {
    pub fn new() -> Self {
        let mut buffer = Vec::with_capacity(BUFFER_SIZE);
        for _ in 0..BUFFER_SIZE {
            buffer.push(atomic::AtomicPtr::new(std::ptr::null_mut()));
        }

        Self {
            buffer,
            head: atomic::AtomicUsize::new(0),
        }
    }

    pub fn is_empty(&self) -> bool {
        let current = self.head.load(atomic::Ordering::Acquire);
        current < 1
    }

    pub fn insert(&self, ptr: *mut T) -> Result<(), *mut T> {
        let current = self.head.load(atomic::Ordering::Acquire);
        let next = current + 1;
        if next >= BUFFER_SIZE {
            return Err(ptr);
        }

        self.head.store(next, atomic::Ordering::Release);

        let bucket = unsafe { self.buffer.get_unchecked(current) };
        match bucket.compare_exchange(
            std::ptr::null_mut(),
            ptr,
            atomic::Ordering::SeqCst,
            atomic::Ordering::SeqCst,
        ) {
            Ok(_) => Ok(()),
            Err(_) => Err(ptr),
        }
    }

    pub fn pop(&self) -> Option<*mut T> {
        let current = self.head.load(atomic::Ordering::Acquire);
        if current < 1 {
            return None;
        }

        let next = current - 1;
        self.head.store(next, atomic::Ordering::Release);

        let bucket = unsafe { self.buffer.get_unchecked(next) };
        let ptr = bucket.load(atomic::Ordering::Acquire);

        match bucket.compare_exchange(
            ptr,
            std::ptr::null_mut(),
            atomic::Ordering::SeqCst,
            atomic::Ordering::SeqCst,
        ) {
            Ok(_) => Some(ptr),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_new() {
        let buffer = AllocationBuffer::<usize>::new();
        drop(buffer);
    }

    #[test]
    fn buffer_insert() {
        let buffer = AllocationBuffer::<usize>::new();

        buffer.insert(123 as *mut usize).unwrap();
    }

    #[test]
    fn buffer_insert_pop() {
        let buffer = AllocationBuffer::<usize>::new();

        buffer.insert(123 as *mut usize).unwrap();

        let result = buffer.pop();
        assert_eq!(Some(123 as *mut usize), result);
    }

    #[test]
    fn buffer_pop_empty() {
        let buffer = AllocationBuffer::<usize>::new();

        assert_eq!(None, buffer.pop());
    }

    #[test]
    fn buffer_multiple_inserts() {
        let buffer = AllocationBuffer::<usize>::new();

        buffer.insert(123 as *mut usize).unwrap();
        assert_eq!(Some(123 as *mut usize), buffer.pop());

        buffer.insert(234 as *mut usize).unwrap();
        assert_eq!(Some(234 as *mut usize), buffer.pop());
    }

    #[test]
    fn buffer_is_empty() {
        let buffer = AllocationBuffer::<usize>::new();

        assert_eq!(true, buffer.is_empty());

        buffer.insert(123 as *mut usize).unwrap();
        assert_eq!(false, buffer.is_empty());
    }
}
