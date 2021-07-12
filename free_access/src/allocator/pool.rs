//! This represents the Pool implementation for the Allocation-Pool
//!
//! # Strucure
//! The Pool consists of a doubly-linked List of Nodes, which will never be
//! deallocated to make sure that we never access a removed Node.
//! Instead a Node can be in one of three Stages
//!
//! ## Stages
//! * Empty: The Node contains no Data
//! * Accessed: The Node is currently being modified, this prevents multiple
//! Threads from trying to obtain the same Data
//! * Set: The Node contains some Data that is ready to be read
//!
//! ## Access-Pattern
//! Because the Pool needs to be protected using the current Phase, but rust
//! currently does not support 128-bit Atomics, we need to find a way around
//! that. For this purpose we will check the Phase once at the beginning of an
//! operation, to filter out wrong phases as quickly as possible, and then
//! again once the Node was set to the Accessed state.
//! Each Node also holds the Phase of when it was set, this allows us to
//! overwirite the Node if we notice that it has been set in an old version.
//!
//! ### Push
//! ```pseudo
//! push(local_phase, data):
//!     if pool.Phase != local_phase:
//!         return
//!
//!     for each Node in the Stack:
//!         if Node.State == EMPTY:
//!             if !CAS(Node.State, EMPTY, ACCESSED):
//!                 continue
//!             if pool.Phase != local_phase:
//!                 Node.State = EMPTY
//!                 return;
//!
//!             Node.Data = data
//!             Node.Phase = local_phase
//!             Node.State = SET
//!             return
//!         if Node.State == SET:
//!             if !CAS(Node.State, SET, ACCESSED):
//!                 continue
//!             if Node.Phase != local_phase:
//!                 Node.State = SET
//!                 continue
//!             if pool.Phase != local_phase:
//!                 Node.State = SET
//!                 return
//!
//!             Clear(Node)
//!             Node.Data = data
//!             Node.Phase = local_phase
//!             Node.State = SET
//!             return
//! ```
//!
//! ### Pop
//! ```pseudo
//! pop(local_phase):
//!     if pool.Phase != local_phase:
//!         return
//!
//!     for each Node in the Stack:
//!         if Node.State == SET:
//!             if !CAS(Node.State, SET, ACCESSED):
//!                 continue
//!             if Node.Phase != pool.Phase:
//!                 Clear(Node)
//!                 continue
//!             if pool.Phase != local_phase:
//!                 Node.State = SET
//!                 return;
//!             
//!             data = Node.Data
//!             Node.State = EMPTY
//!             return Data
//! ```

use std::{cell::UnsafeCell, mem::MaybeUninit, sync::atomic};

enum State {
    Empty,
    Accessed,
    Set,
}

impl State {
    pub const fn to_u8(&self) -> u8 {
        match self {
            Self::Empty => 0,
            Self::Accessed => 1,
            Self::Set => 2,
        }
    }
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Empty),
            1 => Some(Self::Accessed),
            2 => Some(Self::Set),
            _ => None,
        }
    }
}

struct Node<T> {
    data: UnsafeCell<MaybeUninit<T>>,
    state: atomic::AtomicU8,
    next: atomic::AtomicPtr<Self>,
    // Add a phase counter
}

impl<T> Node<T> {
    pub fn new() -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::uninit()),
            state: atomic::AtomicU8::new(State::Empty.to_u8()),
            next: atomic::AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    pub fn load_state(&self, order: atomic::Ordering) -> State {
        let raw = self.state.load(order);
        State::from_u8(raw).unwrap()
    }
}

/// The Pool is intended as a Stack-Like Datastructure, which is phase
/// protected.
/// This Datastructure does not provide any garantuees for the Order of
/// Elements being inserted/returned
pub struct Pool<T> {
    /// The current Phase
    phase: atomic::AtomicU64,
    /// The First Element of the List of Nodes
    start: *mut Node<T>,
}

impl<T> Pool<T> {
    pub fn new() -> Self {
        let initial_node_ptr = Box::into_raw(Box::new(Node::new()));

        Self {
            phase: atomic::AtomicU64::new(0),
            start: initial_node_ptr,
        }
    }

    pub fn update_phase(&self, n_phase: u64) -> Result<(), ()> {
        let mut previous = self.phase.load(atomic::Ordering::Acquire);
        loop {
            if previous >= n_phase {
                return Err(());
            }

            match self.phase.compare_exchange(
                previous,
                n_phase,
                atomic::Ordering::SeqCst,
                atomic::Ordering::SeqCst,
            ) {
                Ok(_) => return Ok(()),
                Err(cur) => {
                    previous = cur;
                }
            };
        }
    }

    pub fn insert(&self, data: T, phase: u64) -> Result<(), ()> {
        if self.phase.load(atomic::Ordering::Acquire) != phase {
            return Err(());
        }

        let mut current = unsafe { &*self.start };

        // Attempt to find
        loop {
            if let State::Empty = current.load_state(atomic::Ordering::Acquire) {
                match current.state.compare_exchange(
                    State::Empty.to_u8(),
                    State::Accessed.to_u8(),
                    atomic::Ordering::SeqCst,
                    atomic::Ordering::SeqCst,
                ) {
                    Ok(_) => {
                        if self.phase.load(atomic::Ordering::Acquire) != phase {
                            current
                                .state
                                .store(State::Empty.to_u8(), atomic::Ordering::Release);
                            return Err(());
                        }

                        let data_ptr = current.data.get() as *mut T;
                        unsafe { data_ptr.write(data) };

                        current
                            .state
                            .store(State::Set.to_u8(), atomic::Ordering::Release);
                        return Ok(());
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

        let next_node = Node::new();
        next_node
            .state
            .store(State::Accessed.to_u8(), atomic::Ordering::Release);
        let next_ptr = Box::into_raw(Box::new(next_node));

        loop {
            match current.next.compare_exchange(
                std::ptr::null_mut(),
                next_ptr,
                atomic::Ordering::SeqCst,
                atomic::Ordering::SeqCst,
            ) {
                Ok(_) => {
                    let next_node = unsafe { &*next_ptr };
                    if self.phase.load(atomic::Ordering::Acquire) != phase {
                        next_node
                            .state
                            .store(State::Empty.to_u8(), atomic::Ordering::Release);
                        return Err(());
                    }

                    return Ok(());
                }
                Err(next) => {
                    current = unsafe { &*next };
                }
            };
        }
    }

    pub fn pop(&self, phase: u64) -> Result<T, ()> {
        if self.phase.load(atomic::Ordering::Acquire) != phase {
            return Err(());
        }

        let mut current = unsafe { &*self.start };
        loop {
            if let State::Set = current.load_state(atomic::Ordering::Acquire) {
                match current.state.compare_exchange(
                    State::Set.to_u8(),
                    State::Accessed.to_u8(),
                    atomic::Ordering::SeqCst,
                    atomic::Ordering::SeqCst,
                ) {
                    Ok(_) => {
                        if self.phase.load(atomic::Ordering::Acquire) != phase {
                            current
                                .state
                                .store(State::Set.to_u8(), atomic::Ordering::Release);
                            return Err(());
                        }

                        let data_ptr = current.data.get();

                        let data = unsafe { data_ptr.read().assume_init() };
                        unsafe { data_ptr.write(MaybeUninit::uninit()) };

                        return Ok(data);
                    }
                    Err(_) => {}
                };
            }

            let next_ptr = current.next.load(atomic::Ordering::Acquire);
            if next_ptr.is_null() {
                break;
            }

            current = unsafe { &*next_ptr };
        }

        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_new() {
        let pool = Pool::<usize>::new();
        drop(pool);
    }

    #[test]
    fn pool_insert() {
        let pool = Pool::<usize>::new();

        assert_eq!(Ok(()), pool.insert(13, 0));
    }
    #[test]
    fn pool_insert_wrong_phase() {
        let pool = Pool::<usize>::new();

        assert_eq!(Ok(()), pool.insert(13, 0));

        pool.update_phase(1);
        assert_eq!(Err(()), pool.insert(13, 0));
    }
    #[test]
    fn pool_insert_multiple() {
        let pool = Pool::<usize>::new();

        assert_eq!(Ok(()), pool.insert(13, 0));
        assert_eq!(Ok(()), pool.insert(14, 0));
        assert_eq!(Ok(()), pool.insert(15, 0));
    }

    #[test]
    fn pool_insert_pop() {
        let pool = Pool::<usize>::new();

        assert_eq!(Ok(()), pool.insert(13, 0));

        assert_eq!(Ok(13), pool.pop(0));
    }
}
