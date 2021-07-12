use std::sync::atomic;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct DirtyValue {
    pub dirty: bool,
    pub phase: u64,
}
impl DirtyValue {
    pub const fn from_u64(val: u64) -> Self {
        let dirty = val & 1 == 1;
        let phase = val >> 8;

        Self { dirty, phase }
    }

    pub const fn to_u64(&self) -> u64 {
        let result = self.phase << 8;
        let dirty_mask: u64 = if self.dirty { 0x01 } else { 0x00 };
        result | dirty_mask
    }
}

pub struct Udirty {
    dirty_phase: atomic::AtomicU64,
}
impl Udirty {
    pub fn new() -> Self {
        Self {
            dirty_phase: atomic::AtomicU64::new(0),
        }
    }

    pub fn get(&self) -> DirtyValue {
        let raw = self.dirty_phase.load(atomic::Ordering::Acquire);
        DirtyValue::from_u64(raw)
    }

    pub fn update(&self, expected: u64, n_dirty: DirtyValue) -> bool {
        let raw = n_dirty.to_u64();

        self.dirty_phase
            .compare_exchange(
                expected,
                raw,
                atomic::Ordering::SeqCst,
                atomic::Ordering::SeqCst,
            )
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_value_from_u64() {
        let raw: u64 = 0x1201;
        assert_eq!(
            DirtyValue {
                phase: 0x12,
                dirty: true,
            },
            DirtyValue::from_u64(raw)
        );

        let raw: u64 = 0x2100;
        assert_eq!(
            DirtyValue {
                phase: 0x21,
                dirty: false,
            },
            DirtyValue::from_u64(raw)
        );
    }

    #[test]
    fn dirty_value_to_u64() {
        let dirty = DirtyValue {
            dirty: true,
            phase: 0x12,
        };
        assert_eq!(0x1201, dirty.to_u64());

        let dirty = DirtyValue {
            dirty: false,
            phase: 0x21,
        };
        assert_eq!(0x2100, dirty.to_u64());
    }
}
