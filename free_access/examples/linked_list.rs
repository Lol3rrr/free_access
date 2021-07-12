use std::sync::{atomic, Arc};

pub struct ListNode<T> {
    data: T,
    next: atomic::AtomicPtr<Self>,
}

pub struct LinkedListGlobal<T> {
    ptr: Arc<atomic::AtomicPtr<ListNode<T>>>,
}

pub struct LinkedList<T> {
    allocator: free_access::Allocator<ListNode<T>, LinkedListGlobal<T>>,
    head: Arc<atomic::AtomicPtr<ListNode<T>>>,
}

impl<T> free_access::DataStructureGlobals<ListNode<T>> for LinkedListGlobal<T> {
    fn get_globals(&self) -> Vec<*mut ListNode<T>> {
        vec![self.ptr.load(atomic::Ordering::Acquire)]
    }
}

impl<T> free_access::DataStructureNode for ListNode<T> {
    fn pointer_count() -> usize {
        1
    }
    fn pointers(&self) -> Vec<*mut Self> {
        vec![self.next.load(atomic::Ordering::Acquire)]
    }

    fn untag_ptr(ptr: *mut Self) -> *mut Self {
        ptr
    }
}

impl<T> LinkedList<T> {
    pub fn new() -> Self {
        let head = Arc::new(atomic::AtomicPtr::new(std::ptr::null_mut()));

        Self {
            allocator: free_access::Allocator::new(LinkedListGlobal { ptr: head.clone() }),
            head,
        }
    }

    #[tracing::instrument(skip(self, data))]
    pub fn append(&self, data: T) {
        let new_node = ListNode {
            data,
            next: atomic::AtomicPtr::new(std::ptr::null_mut()),
        };
        let allocated = self.allocator.allocate(new_node);
        tracing::debug!("New-Node: {:p}", allocated.ptr());

        let mut head = self.head.load(atomic::Ordering::Acquire);
        if head.is_null() {
            match self.head.compare_exchange(
                std::ptr::null_mut(),
                allocated.ptr(),
                atomic::Ordering::SeqCst,
                atomic::Ordering::SeqCst,
            ) {
                Ok(_) => {
                    return;
                }
                Err(n_head) => {
                    head = n_head;
                }
            };
        }

        let mut current = unsafe { &*head };
        loop {
            let mut next = current.next.load(atomic::Ordering::Acquire);
            if next.is_null() {
                match current.next.compare_exchange(
                    std::ptr::null_mut(),
                    allocated.ptr(),
                    atomic::Ordering::SeqCst,
                    atomic::Ordering::SeqCst,
                ) {
                    Ok(_) => return,
                    Err(n_next) => {
                        next = n_next;
                    }
                };
            }

            current = unsafe { &*next };
        }
    }
}

fn main() {
    tracing_subscriber::fmt::init();

    let list: LinkedList<u64> = LinkedList::new();

    list.append(13);
    list.append(14);

    list.allocator.force_gc();
}
