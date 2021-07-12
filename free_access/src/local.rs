use std::sync::atomic;

use crate::{
    allocator::{NodeMarks, Page},
    DataStructureNode,
};

use super::{allocator, markstack, Arbiter, HazardPtrFrame, Udirty};

pub struct Local<T> {
    pub thread_id: std::thread::ThreadId,
    pub phase_index: atomic::AtomicU64,
    pub dirty: Udirty,
    pub hazard_ptr_frames: [HazardPtrFrame<T>; 2],
    // Either 0 or 1
    pub(crate) arbiter: Arbiter,
    pub alloc: allocator::LocalAllocator<T>,

    // Marking stuff
    pub cur_traced: atomic::AtomicPtr<T>,
    pub mark_stack: markstack::MarkStack<T>,
}

impl<T> Default for Local<T> {
    fn default() -> Self {
        Self {
            thread_id: std::thread::current().id(),
            phase_index: atomic::AtomicU64::new(0),
            dirty: Udirty::new(),
            hazard_ptr_frames: [HazardPtrFrame::new(), HazardPtrFrame::new()],
            arbiter: Arbiter::new(),
            alloc: allocator::LocalAllocator::new(),
            cur_traced: atomic::AtomicPtr::new(std::ptr::null_mut()),
            mark_stack: markstack::MarkStack::new(),
        }
    }
}

pub enum MarkNodeState {
    Done,
    NotDone,
}

impl<T> Local<T>
where
    T: DataStructureNode,
{
    #[tracing::instrument(skip(self))]
    pub fn mark_node(&self, local_phase: u64) -> MarkNodeState {
        let obj_ptr = match self.mark_stack.peek() {
            Some(o) => o,
            None => {
                tracing::debug!("Marking Done");
                return MarkNodeState::Done;
            }
        };

        tracing::debug!("Marking Node: {:p}", obj_ptr);

        let obj_node = unsafe { allocator::PageNode::from_data_ptr(obj_ptr) };
        let marks = obj_node.load_marks();
        if marks.marked || marks.phase != local_phase {
            tracing::debug!("Already marked or wrong phase: {:?}", marks);

            self.mark_stack.pop();
            return MarkNodeState::NotDone;
        }

        self.cur_traced.store(obj_ptr, atomic::Ordering::Release);
        let _ = self.mark_stack.pop();

        let mut pushed_children = 0;
        let obj = unsafe { &*obj_ptr };
        let child_ptrs = obj.pointers();
        for c_ptr in child_ptrs {
            if c_ptr.is_null() {
                continue;
            }
            self.mark_stack.push(c_ptr);
            pushed_children += 1;
        }

        let expected_marks = NodeMarks {
            phase: local_phase,
            marked: false,
        };
        let new_marks = NodeMarks {
            phase: local_phase,
            marked: true,
        };
        match obj_node.update_marks(expected_marks, new_marks) {
            Ok(_) => MarkNodeState::NotDone,
            Err(_) => {
                for _ in 0..pushed_children {
                    let _ = self.mark_stack.pop();
                }
                MarkNodeState::NotDone
            }
        }
    }

    #[tracing::instrument(skip(self, page, global_alloc))]
    pub fn sweep_page(&self, page: &Page<T>, global_alloc: &allocator::GlobalAllocPool<T>) {
        let local_phase = self.phase_index.load(atomic::Ordering::Acquire);

        tracing::debug!(local_phase, "Sweeping Page");

        for node in page.nodes.iter() {
            let marks = node.load_marks();
            if marks.marked {
                continue;
            }

            let data_ptr = unsafe { node.get_data_ptr() };
            match self.alloc.insert(data_ptr) {
                Ok(_) => {}
                Err(data_ptr) => {
                    let old = self.alloc.take();
                    let _ = global_alloc.insert(local_phase, old);

                    self.alloc.insert(data_ptr).expect("");
                }
            };
        }
    }
}
