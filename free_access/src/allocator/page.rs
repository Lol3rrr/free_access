use std::sync::atomic;

#[derive(Debug, Clone, PartialEq)]
pub struct NodeMarks {
    pub marked: bool,
    pub phase: u64,
}

impl From<u64> for NodeMarks {
    fn from(raw: u64) -> Self {
        let marked = raw & 0x01 == 0x01;
        let phase = raw >> 8;
        Self { marked, phase }
    }
}
impl Into<u64> for NodeMarks {
    fn into(self) -> u64 {
        let marked_mask = if self.marked { 0x01 } else { 0x00 };
        let result = ((self.phase << 8) & 0xffffffffffffff00) | marked_mask;
        result
    }
}

mod node;
pub use node::PageNode;

pub struct Page<T> {
    pub nodes: Vec<PageNode<T>>,
    next: atomic::AtomicPtr<Self>,
}

impl<T> Page<T> {
    pub fn new(size: usize) -> Self {
        let mut nodes = Vec::with_capacity(size);
        for _ in 0..size {
            nodes.push(PageNode::new());
        }

        Self {
            nodes,
            next: atomic::AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn update_marks(&self, n_phase: u64) {
        tracing::debug!("Updating-Marks");
        for node in self.nodes.iter() {
            node.clear_marks(n_phase);
        }
    }
}

pub struct PageList<T> {
    page_size: usize,
    head: *mut Page<T>,
    page_count: atomic::AtomicU64,
}

impl<T> PageList<T> {
    pub fn new(page_size: usize) -> Self {
        let initial_page = Box::into_raw(Box::new(Page::new(page_size)));

        Self {
            page_size,
            head: initial_page,
            page_count: atomic::AtomicU64::new(1),
        }
    }

    fn get_page_index<'a>(&self, index: u64) -> Option<&'a Page<T>> {
        if index >= self.page_count.load(atomic::Ordering::Acquire) {
            return None;
        }

        let mut current = unsafe { &*self.head };
        for _ in 0..index {
            let next = current.next.load(atomic::Ordering::Acquire);
            current = unsafe { &*next };
        }

        Some(current)
    }

    /// Returns the information about the Index in the Format (Phase, Index)
    fn index_data(index: u64) -> (u64, u64) {
        ((index >> 32), (index & 0x00000000ffffffff))
    }

    #[tracing::instrument(skip(self, sweep_chunk_index))]
    pub fn get_page<'a>(
        &self,
        sweep_chunk_index: &atomic::AtomicU64,
        local_phase: u64,
    ) -> Option<&'a Page<T>> {
        let num_sweep_pages = self.page_count.load(atomic::Ordering::Acquire);

        let mut old;
        let mut new;
        loop {
            old = sweep_chunk_index.load(atomic::Ordering::Acquire);
            let (phase, index) = Self::index_data(old);
            if index >= num_sweep_pages {
                return None;
            }
            if phase != local_phase {
                return None;
            }

            new = old + 1;

            if let Ok(_) = sweep_chunk_index.compare_exchange(
                old,
                new,
                atomic::Ordering::SeqCst,
                atomic::Ordering::SeqCst,
            ) {
                return self.get_page_index(index);
            }
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn update_marks(&self, n_phase: u64) {
        let mut current = unsafe { &*self.head };
        loop {
            current.update_marks(n_phase);

            let next = current.next.load(atomic::Ordering::Acquire);
            if next.is_null() {
                break;
            }
            current = unsafe { &*next };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_unmarked() {
        let marked = NodeMarks {
            marked: false,
            phase: 13,
        };

        let serialized: u64 = marked.clone().into();

        assert_eq!(marked, NodeMarks::from(serialized));
    }
    #[test]
    fn marks_marked() {
        let marked = NodeMarks {
            marked: true,
            phase: 13,
        };

        let serialized: u64 = marked.clone().into();

        assert_eq!(marked, NodeMarks::from(serialized));
    }
}
