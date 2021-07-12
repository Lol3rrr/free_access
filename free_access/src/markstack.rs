//! The MarkStack is used during the Mark-Stage of a GC-Phase
//!
//! # Accesses
//! The MarkStack will be read and modified by the current Thread and can also
//! be read by all other Threads, to help in case one Thread get's stuck
//! somewhere.
//!
//! # Memory-Managment
//! The individual Nodes will never be freed/reclaimed and only be reused, this
//! allows us to not worry about whether or not the currently visited Note is
//! still allocated/alive

use std::sync::atomic;

struct StackNode<T> {
    data: atomic::AtomicPtr<T>,
    previous: *mut Self,
    next: atomic::AtomicPtr<Self>,
}

impl<T> StackNode<T> {
    pub fn new(previous: *mut Self, data: *mut T) -> Self {
        Self {
            data: atomic::AtomicPtr::new(data),
            previous,
            next: atomic::AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    pub fn empty() -> Self {
        Self::new(std::ptr::null_mut(), std::ptr::null_mut())
    }
}

pub struct MarkStack<T> {
    head: atomic::AtomicPtr<StackNode<T>>,
}

impl<T> MarkStack<T> {
    pub fn new() -> Self {
        let initial_ptr = Box::into_raw(Box::new(StackNode::empty()));

        Self {
            head: atomic::AtomicPtr::new(initial_ptr),
        }
    }

    pub fn push(&self, data: *mut T) {
        let head_ptr = self.head.load(atomic::Ordering::Acquire);
        let mut current = unsafe { &*head_ptr };

        loop {
            if current.data.load(atomic::Ordering::Acquire).is_null() {
                match current.data.compare_exchange(
                    std::ptr::null_mut(),
                    data,
                    atomic::Ordering::SeqCst,
                    atomic::Ordering::SeqCst,
                ) {
                    Ok(_) => return,
                    Err(_) => {}
                };
            }

            let next = current.next.load(atomic::Ordering::Acquire);
            if next.is_null() {
                break;
            }

            current = unsafe { &*next };
        }

        let current_ptr = current as *const StackNode<T> as *mut StackNode<T>;
        let next_node_ptr = Box::into_raw(Box::new(StackNode::new(current_ptr, data)));
        let next_node = unsafe { &mut *next_node_ptr };

        loop {
            match current.next.compare_exchange(
                std::ptr::null_mut(),
                next_node_ptr,
                atomic::Ordering::SeqCst,
                atomic::Ordering::SeqCst,
            ) {
                Ok(_) => {
                    self.head.store(next_node_ptr, atomic::Ordering::Release);
                    return;
                }
                Err(next) => {
                    next_node.previous = next;
                    current = unsafe { &*next };
                }
            };
        }
    }

    pub fn pop(&self) -> Option<*mut T> {
        let head_ptr = self.head.load(atomic::Ordering::Acquire);
        let mut current = unsafe { &*head_ptr };

        loop {
            let data_ptr = current.data.load(atomic::Ordering::Acquire);
            if !data_ptr.is_null() {
                match current.data.compare_exchange(
                    data_ptr,
                    std::ptr::null_mut(),
                    atomic::Ordering::SeqCst,
                    atomic::Ordering::SeqCst,
                ) {
                    Ok(_) => {
                        let previous = current.previous;
                        if !previous.is_null() {
                            self.head.store(previous, atomic::Ordering::Release);
                        }

                        return Some(data_ptr);
                    }
                    Err(_) => {}
                };
            }

            if current.previous.is_null() {
                return None;
            }

            current = unsafe { &*current.previous };
        }
    }

    pub fn peek(&self) -> Option<*mut T> {
        let head_ptr = self.head.load(atomic::Ordering::Acquire);
        let mut current = unsafe { &*head_ptr };

        loop {
            let data_ptr = current.data.load(atomic::Ordering::Acquire);
            if !data_ptr.is_null() {
                return Some(data_ptr);
            }

            if current.previous.is_null() {
                return None;
            }

            current = unsafe { &*current.previous };
        }
    }

    pub fn is_empty(&self) -> bool {
        let head_ptr = self.head.load(atomic::Ordering::Acquire);
        let mut current = unsafe { &*head_ptr };

        loop {
            let data_ptr = current.data.load(atomic::Ordering::Acquire);
            if !data_ptr.is_null() {
                return false;
            }

            let previous = current.previous;
            if previous.is_null() {
                return true;
            }

            current = unsafe { &*previous };
        }
    }

    pub fn iter(&self) -> MarkStackIter<T> {
        let mut current = unsafe { &*self.head.load(atomic::Ordering::Acquire) };
        loop {
            if current.previous.is_null() {
                break;
            }
            current = unsafe { &*current.previous };
        }

        let current_ptr = current as *const StackNode<T> as *mut StackNode<T>;
        MarkStackIter {
            current: current_ptr,
        }
    }
}

pub struct MarkStackIter<T> {
    current: *mut StackNode<T>,
}

impl<T> Iterator for MarkStackIter<T> {
    type Item = *mut T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_null() {
            return None;
        }

        let current = unsafe { &*self.current };
        let data = current.data.load(atomic::Ordering::Acquire);
        if data.is_null() {
            return None;
        }

        let next = current.next.load(atomic::Ordering::Acquire);
        self.current = next;

        Some(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new() {
        let stack = MarkStack::<usize>::new();
        drop(stack);
    }

    #[test]
    fn is_empty() {
        let stack = MarkStack::<usize>::new();

        assert_eq!(true, stack.is_empty());

        stack.push(0x12 as *mut usize);
        assert_eq!(false, stack.is_empty());

        stack.pop().unwrap();
        assert_eq!(true, stack.is_empty());
    }

    #[test]
    fn pop_empty() {
        let stack = MarkStack::<usize>::new();

        assert_eq!(None, stack.pop());
    }

    #[test]
    fn push_pop() {
        let stack = MarkStack::<usize>::new();

        stack.push(0x12 as *mut usize);
        assert_eq!(Some(0x12 as *mut usize), stack.pop());
    }

    #[test]
    fn push_multiple() {
        let stack = MarkStack::<usize>::new();

        for tmp in 0..10 {
            stack.push(tmp as *mut usize);
        }
        for tmp in 10..0 {
            assert_eq!(Some(tmp as *mut usize), stack.pop());
        }
    }

    #[test]
    fn peek() {
        let stack = MarkStack::<usize>::new();

        assert_eq!(None, stack.peek());

        stack.push(0x12 as *mut usize);
        assert_eq!(Some(0x12 as *mut usize), stack.peek());
        assert_eq!(Some(0x12 as *mut usize), stack.peek());

        assert_eq!(Some(0x12 as *mut usize), stack.pop());
        assert_eq!(None, stack.peek());
    }

    #[test]
    fn iterator() {
        let stack = MarkStack::<usize>::new();

        stack.push(0x12 as *mut usize);
        stack.push(0x23 as *mut usize);

        let mut iter = stack.iter();

        assert_eq!(Some(0x12 as *mut usize), iter.next());
        assert_eq!(Some(0x23 as *mut usize), iter.next());
        assert_eq!(None, iter.next());
    }
}
