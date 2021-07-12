#![deny(missing_docs)]
#![warn(rust_2018_idioms)]
//! TODO
//!
//! # General Strucuture
//!

use allocator::PageList;
pub use free_access_macros::freeaccess;
use thread_local::ThreadLocal;

use std::{
    collections::{HashMap, HashSet},
    sync::atomic,
};

mod dirty;
use dirty::{DirtyValue, Udirty};

mod hazard_ptrs;
use hazard_ptrs::HazardPtrFrame;

mod allocator;
mod markstack;

struct Arbiter(atomic::AtomicU8);
impl Arbiter {
    pub fn new() -> Self {
        Self(atomic::AtomicU8::new(0))
    }

    pub fn get(&self) -> u8 {
        self.0.load(atomic::Ordering::Acquire)
    }

    pub fn next(&self) -> u8 {
        let prev = self.0.load(atomic::Ordering::Acquire);
        prev % 2
    }

    pub fn store(&self, n_val: u8) {
        self.0.store(n_val, atomic::Ordering::Release);
    }
}

mod local;
use local::{Local, MarkNodeState};

/// The Allocator that should be used to allocate/create Nodes of the
/// Datastructure
pub struct Allocator<T, G> {
    phase_index: atomic::AtomicU64,
    local: ThreadLocal<Local<T>>,
    allocation_pool: allocator::GlobalAllocPool<T>,
    pages: PageList<T>,
    sweep_chunk_index: atomic::AtomicU64,
    globals: G,
}

/// This is very similiar to the Standard Box with the main Difference being
/// that this "Box" is tied to the Allocator and does not free the Memory
/// to the OS itself when dropped
pub struct AoaBox<T> {
    inner: *mut T,
}

impl<T> AoaBox<T> {
    /// TODO
    pub fn ptr(&self) -> *mut T {
        self.inner
    }
}

impl<N, G> Allocator<N, G>
where
    N: DataStructureNode,
    G: DataStructureGlobals<N>,
{
    /// TODO
    #[tracing::instrument(skip(globals))]
    pub fn new(globals: G) -> Self {
        tracing::debug!("Creating new Allocator");

        let result = Self {
            phase_index: atomic::AtomicU64::new(0),
            local: ThreadLocal::new(),
            allocation_pool: allocator::GlobalAllocPool::new(),
            pages: PageList::new(256),
            sweep_chunk_index: atomic::AtomicU64::new(0),
            globals,
        };

        result.sweep();

        result
    }

    /// Actually allocates the given Data
    #[tracing::instrument(skip(self, data))]
    pub fn allocate(&self, data: N) -> AoaBox<N> {
        tracing::debug!("Allocating");

        let local = self.local.get_or_default();
        if local.alloc.is_empty() {
            let lphase_index = local.phase_index.load(atomic::Ordering::Acquire);

            match self.allocation_pool.pop(lphase_index) {
                Ok(n_buffer) => {
                    local.alloc.new_buffer(n_buffer);
                }
                Err(_) => {
                    todo!()
                }
            };
        }

        let ptr = local.alloc.pop().unwrap();

        unsafe { ptr.write(data) };
        AoaBox { inner: ptr }
    }

    /// Forces the Allocator to start a Garbage-Collection Phase
    pub fn force_gc(&self) {
        self.reclaimation();
    }

    /// TODO
    pub fn restart(&self) {
        // TODO
        // Help reclaimation

        let locals = self.local.get_or_default();
        todo!()
    }

    /// This is used to attempt the Start of a Write-Only Period, if this
    /// Returns an Error, the execution should be restarted at the previous
    /// Read-Only Period
    ///
    /// # Parameters
    /// * `local_ptrs`: The currently loaded Ptrs that will be used in the next
    /// Stage, these should still contain the Tags, if the Datastructure uses
    /// Tags (they should not be cleared here)
    pub fn begin_write_only(&self, local_ptrs: &[*mut N]) -> Result<(), ()> {
        let locals = self.local.get_or_default();

        let next_arbiter = locals.arbiter.next();

        let hazard_ptr_frame = &locals.hazard_ptr_frames[next_arbiter as usize];
        for p in local_ptrs {
            hazard_ptr_frame.store(*p);
        }

        let dirty = locals.dirty.get();
        if dirty.dirty {
            return Err(());
        }

        locals.arbiter.store(next_arbiter);
        Ok(())
    }

    /// This validates that a Value read from some Address is valid, this
    /// should be called before using the Value's read
    pub fn validate_read(&self) -> Result<(), ()> {
        let local = self.local.get_or_default();
        let dirty = local.dirty.get();
        if dirty.dirty {
            Err(())
        } else {
            Ok(())
        }
    }

    fn local_roots(&self) -> Vec<*mut N> {
        let mut result = Vec::new();

        for t in self.local.iter() {
            result.extend(
                t.hazard_ptr_frames[0]
                    .roots()
                    .into_iter()
                    .map(|ptr| N::untag_ptr(ptr)),
            );
            result.extend(
                t.hazard_ptr_frames[1]
                    .roots()
                    .into_iter()
                    .map(|ptr| N::untag_ptr(ptr)),
            );
        }

        result
    }

    fn global_roots(&self) -> Vec<*mut N> {
        self.globals.get_globals()
    }

    fn gather_roots(&self) -> Vec<*mut N> {
        let mut result = self.local_roots();
        result.extend(self.global_roots());

        result
    }

    fn help(&self, local: &local::Local<N>, node: *mut N) {
        if local.phase_index.load(atomic::Ordering::Acquire)
            == self.phase_index.load(atomic::Ordering::Acquire)
        {
            local.mark_stack.push(node);
        } else {
            todo!("Clear MarkStack")
        }
    }

    #[tracing::instrument(skip(self))]
    fn finish_or_progress(&self) -> bool {
        let mut threads: HashSet<std::thread::ThreadId> = HashSet::new();
        let mut cur_phase: HashMap<std::thread::ThreadId, u64> = HashMap::new();
        let mut cur_traces: HashMap<std::thread::ThreadId, *mut N> = HashMap::new();

        let own_local = self.local.get_or_default();
        let local_phase = own_local.phase_index.load(atomic::Ordering::Acquire);

        tracing::debug!("First Block");
        for tmp_local in self.local.iter() {
            let local_thread_id = &tmp_local.thread_id;

            let tmp_phase = tmp_local.phase_index.load(atomic::Ordering::Acquire);
            let tmp_cur_traced = tmp_local.cur_traced.load(atomic::Ordering::Acquire);
            if tmp_cur_traced.is_null() {
                continue;
            }

            threads.insert(local_thread_id.clone());
            cur_phase.insert(local_thread_id.clone(), tmp_phase);
            cur_traces.insert(local_thread_id.clone(), tmp_cur_traced);

            let obj_node = unsafe { allocator::PageNode::from_data_ptr(tmp_cur_traced) };
            let marks = obj_node.load_marks();
            if tmp_phase == local_phase && !marks.marked {
                self.help(own_local, tmp_cur_traced);
                return false;
            }
        }

        tracing::debug!("Second Block");
        for tmp_local in self.local.iter() {
            let tmp_thread_id = &tmp_local.thread_id;
            if !threads.contains(tmp_thread_id) {
                continue;
            }

            if *cur_phase.get(tmp_thread_id).unwrap() != local_phase {
                continue;
            }

            // Iterate over all entries of the MarkStack and help if needed
            let tmp_mark_stack = &tmp_local.mark_stack;
            for node in tmp_mark_stack.iter() {
                let obj_node = unsafe { allocator::PageNode::from_data_ptr(node) };
                let marks = obj_node.load_marks();
                if !marks.marked {
                    self.help(own_local, node);
                    return false;
                }
            }
        }

        tracing::debug!("Third Block");
        for tmp_local in self.local.iter() {
            let tmp_thread_id = &tmp_local.thread_id;
            if !threads.contains(tmp_thread_id) {
                continue;
            }

            if *cur_traces.get(tmp_thread_id).unwrap()
                != tmp_local.cur_traced.load(atomic::Ordering::Acquire)
            {
                return false;
            }
            if *cur_phase.get(tmp_thread_id).unwrap()
                != tmp_local.phase_index.load(atomic::Ordering::Acquire)
            {
                return false;
            }
        }

        true
    }

    #[tracing::instrument(skip(self))]
    fn trace(&self, roots: Vec<*mut N>) {
        tracing::debug!("Tracing");

        let local = self.local.get_or_default();
        let local_phase = local.phase_index.load(atomic::Ordering::Acquire);

        for root in roots {
            local.mark_stack.push(root);
        }

        tracing::debug!("Starting the Trace-Routine");
        loop {
            loop {
                if let MarkNodeState::Done = local.mark_node(local_phase) {
                    break;
                }
            }

            if self.finish_or_progress() {
                break;
            }
        }
    }

    #[tracing::instrument(skip(self))]
    fn sweep(&self) {
        let local = self.local.get_or_default();
        let local_phase = local.phase_index.load(atomic::Ordering::Acquire);

        tracing::debug!(local_phase, "Sweeping");

        loop {
            match self.pages.get_page(&self.sweep_chunk_index, local_phase) {
                Some(page) => local.sweep_page(page, &self.allocation_pool),
                None => {
                    tracing::debug!("Done-Sweeping");
                    return;
                }
            };
        }
    }

    #[tracing::instrument(skip(self))]
    fn reclaimation(&self) {
        tracing::debug!("Starting Reclaimation");

        self.init_reclaimation();

        self.update_marks();
        self.clear_alloc_pools();

        // Gather all Roots
        let roots = self.gather_roots();

        // Trace the Roots
        self.trace(roots);

        // Sweep
        self.sweep();
    }

    #[tracing::instrument(skip(self))]
    fn update_marks(&self) {
        tracing::debug!("Clearing Marks");
        let local = self.local.get_or_default();
        let local_phase = local.phase_index.load(atomic::Ordering::Acquire);

        self.pages.update_marks(local_phase);
    }

    #[tracing::instrument(skip(self))]
    fn clear_alloc_pools(&self) {
        tracing::debug!("Clearing Allocation-Pools");

        // TODO
        let local = self.local.get_or_default();
        let local_phase = local.phase_index.load(atomic::Ordering::Acquire);

        match self.allocation_pool.clear(local_phase) {
            Ok(_) => {
                tracing::debug!("Cleared Global-Allocation Pool");
            }
            Err(_) => {
                tracing::debug!("Could not clear Global-Allocation Pool");
            }
        };
    }

    /// This signals all Threads that a new Phase has started
    #[tracing::instrument(skip(self))]
    fn init_reclaimation(&self) {
        tracing::debug!("Init Reclaimation");

        let local = self.local.get_or_default();
        let lphase_index = local.phase_index.load(atomic::Ordering::Acquire);
        let _ = self.phase_index.compare_exchange(
            lphase_index,
            lphase_index + 1,
            atomic::Ordering::SeqCst,
            atomic::Ordering::SeqCst,
        );

        let nphase_index = self.phase_index.load(atomic::Ordering::Acquire);

        local
            .phase_index
            .store(nphase_index, atomic::Ordering::Release);

        for thread in self.local.iter() {
            let t_dirty = thread.dirty.get();

            // TODO
            // Investigate why the Paper uses a while loop for this
            if t_dirty.phase < lphase_index {
                let expected = t_dirty.to_u64();
                thread.dirty.update(
                    expected,
                    DirtyValue {
                        dirty: true,
                        phase: lphase_index,
                    },
                );
            }
        }
    }
}

/// This trait should be implemented for the actual Node-Type of your
/// Datastructure
pub trait DataStructureNode {
    /// The maximum amount of pointers to other Nodes in a single Node
    fn pointer_count() -> usize;
    /// Actually loads the Pointers from the current Node to others
    fn pointers(&self) -> Vec<*mut Self>;

    /// This gets passed a Ptr that could be tagged and should remove the
    /// Tag from it
    fn untag_ptr(ptr: *mut Self) -> *mut Self;
}

/// TODO
pub trait DataStructureGlobals<N> {
    /// TODO
    fn get_globals(&self) -> Vec<*mut N>;
}
