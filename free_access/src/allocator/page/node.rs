use std::{mem::MaybeUninit, sync::atomic};

use memoffset::offset_of;

use super::NodeMarks;

#[repr(C)]
pub struct PageNode<T> {
    marker: atomic::AtomicU64,
    data: MaybeUninit<T>,
}

impl<T> PageNode<T> {
    pub fn new() -> Self {
        let marks = NodeMarks {
            phase: 0,
            marked: false,
        };
        let mark_value = marks.into();

        Self {
            marker: atomic::AtomicU64::new(mark_value),
            data: MaybeUninit::uninit(),
        }
    }

    fn data_offset() -> usize {
        offset_of!(PageNode<T>, data)
    }

    /// Gets a Ptr to the data field of this node
    pub unsafe fn get_data_ptr(&self) -> *mut T {
        let base_ptr = self as *const Self as usize;
        (base_ptr + Self::data_offset()) as *mut T
    }

    /// Converts the given DataPtr back to a Reference to the underlying
    /// PageNode
    pub unsafe fn from_data_ptr<'a>(ptr: *mut T) -> &'a Self {
        let base_ptr = ((ptr as usize) - Self::data_offset()) as *mut T;
        &*(base_ptr as *mut Self)
    }

    pub fn load_marks(&self) -> NodeMarks {
        let raw_marks = self.marker.load(atomic::Ordering::Acquire);
        raw_marks.into()
    }

    #[tracing::instrument(skip(self))]
    pub fn update_marks(&self, expected: NodeMarks, n_marks: NodeMarks) -> Result<(), ()> {
        let current: u64 = expected.into();
        let new: u64 = n_marks.into();

        match self.marker.compare_exchange(
            current,
            new,
            atomic::Ordering::SeqCst,
            atomic::Ordering::SeqCst,
        ) {
            Ok(_) => Ok(()),
            Err(_) => Err(()),
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn clear_marks(&self, n_phase: u64) {
        let previous_marks_raw = self.marker.load(atomic::Ordering::Acquire);
        let previous_marks = NodeMarks::from(previous_marks_raw);
        if previous_marks.phase >= n_phase {
            tracing::debug!("Previous Phase is newer");
            return;
        }

        let new_marks = NodeMarks {
            phase: n_phase,
            marked: false,
        };

        let new_mark_value: u64 = new_marks.into();

        match self.marker.compare_exchange(
            previous_marks_raw,
            new_mark_value,
            atomic::Ordering::SeqCst,
            atomic::Ordering::SeqCst,
        ) {
            Ok(_) => {}
            Err(previous) => {
                tracing::debug!("Failed clearing Marker");
                tracing::debug!("Current: {:#064b}", previous);
                tracing::debug!("Expected: {:#064b}", previous_marks_raw);
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ptr_stuff() {
        let node = PageNode::<usize>::new();

        let data_ptr = unsafe { node.get_data_ptr() };

        let loaded_node = unsafe { PageNode::from_data_ptr(data_ptr) };

        println!("{:x}", PageNode::<usize>::data_offset());

        assert_eq!(
            (&node) as *const PageNode<usize> as *mut PageNode<usize>,
            loaded_node as *const PageNode<usize> as *mut PageNode<usize>
        );

        assert_eq!(
            node.marker.load(atomic::Ordering::SeqCst),
            loaded_node.marker.load(atomic::Ordering::SeqCst)
        );
    }
}
