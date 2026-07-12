use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Allocation {
    pub page: u16,
    pub offset: u32,
    pub size: u32,
    generation: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ArenaStats {
    pub pages: usize,
    pub capacity_bytes: u64,
    pub allocated_bytes: u64,
}

#[derive(Debug)]
struct Page {
    capacity: u32,
    free: Vec<(u32, u32)>,
}

/// Best-fit, coalescing allocator for fixed-size GPU buffer pages. Allocation metadata is portable
/// and host-tested; the renderer owns the corresponding WGPU buffers.
pub(crate) struct ArenaAllocator {
    default_page_size: u32,
    alignment: u32,
    pages: Vec<Page>,
    active: BTreeMap<(u16, u32), (u32, u32)>,
    next_generation: u32,
}

impl ArenaAllocator {
    pub fn new(default_page_size: u32, alignment: u32) -> Self {
        Self {
            default_page_size: default_page_size.max(1),
            alignment: alignment.max(1),
            pages: Vec::new(),
            active: BTreeMap::new(),
            next_generation: 1,
        }
    }

    pub fn allocate(&mut self, requested_size: u32) -> Option<Allocation> {
        if requested_size == 0 {
            return None;
        }
        let size = align_up(requested_size, self.alignment)?;
        let mut best = None;
        for (page_index, page) in self.pages.iter().enumerate() {
            for (range_index, &(_, free_size)) in page.free.iter().enumerate() {
                if free_size >= size {
                    let candidate = (free_size - size, page_index, range_index);
                    if best.is_none_or(|current| candidate < current) {
                        best = Some(candidate);
                    }
                }
            }
        }
        let (page_index, range_index) = if let Some((_, page, range)) = best {
            (page, range)
        } else {
            if self.pages.len() >= u16::MAX as usize {
                return None;
            }
            let minimum = self.default_page_size.max(size);
            let capacity = minimum.checked_next_power_of_two().unwrap_or(minimum);
            self.pages.push(Page {
                capacity,
                free: vec![(0, capacity)],
            });
            (self.pages.len() - 1, 0)
        };

        let page = self.pages.get_mut(page_index)?;
        let (offset, free_size) = *page.free.get(range_index)?;
        if free_size == size {
            page.free.remove(range_index);
        } else {
            page.free[range_index] = (offset + size, free_size - size);
        }
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let allocation = Allocation {
            page: page_index as u16,
            offset,
            size,
            generation,
        };
        self.active
            .insert((allocation.page, allocation.offset), (size, generation));
        Some(allocation)
    }

    pub fn free(&mut self, allocation: Allocation) -> bool {
        let key = (allocation.page, allocation.offset);
        if self.active.get(&key) != Some(&(allocation.size, allocation.generation)) {
            return false;
        }
        self.active.remove(&key);
        let Some(page) = self.pages.get_mut(allocation.page as usize) else {
            return false;
        };
        page.free.push((allocation.offset, allocation.size));
        page.free.sort_unstable_by_key(|range| range.0);
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(page.free.len());
        for (offset, size) in page.free.drain(..) {
            if let Some(last) = merged.last_mut()
                && last.0 + last.1 == offset
            {
                last.1 += size;
                continue;
            }
            merged.push((offset, size));
        }
        page.free = merged;
        true
    }

    pub fn page_capacity(&self, page: u16) -> Option<u32> {
        self.pages.get(page as usize).map(|value| value.capacity)
    }

    pub fn stats(&self) -> ArenaStats {
        ArenaStats {
            pages: self.pages.len(),
            capacity_bytes: self.pages.iter().map(|page| u64::from(page.capacity)).sum(),
            allocated_bytes: self.active.values().map(|(size, _)| u64::from(*size)).sum(),
        }
    }
}

fn align_up(value: u32, alignment: u32) -> Option<u32> {
    let remainder = value % alignment;
    if remainder == 0 {
        Some(value)
    } else {
        value.checked_add(alignment - remainder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocations_do_not_overlap_and_reuse_best_fit() -> Result<(), &'static str> {
        let mut arena = ArenaAllocator::new(64, 8);
        let first = arena.allocate(13).ok_or("first allocation failed")?;
        let second = arena.allocate(17).ok_or("second allocation failed")?;
        assert_eq!((first.offset, first.size), (0, 16));
        assert_eq!((second.offset, second.size), (16, 24));
        assert!(arena.free(first));
        let reused = arena.allocate(8).ok_or("reused allocation failed")?;
        assert_eq!(reused.offset, 0);
        assert_ne!(reused.generation, first.generation);
        Ok(())
    }

    #[test]
    fn adjacent_ranges_coalesce_after_free() -> Result<(), &'static str> {
        let mut arena = ArenaAllocator::new(64, 4);
        let a = arena.allocate(16).ok_or("first allocation failed")?;
        let b = arena.allocate(16).ok_or("second allocation failed")?;
        let c = arena.allocate(32).ok_or("third allocation failed")?;
        assert!(arena.free(b));
        assert!(arena.free(a));
        assert!(arena.free(c));
        let whole_page = arena.allocate(64).ok_or("coalesced allocation failed")?;
        assert_eq!(
            (whole_page.page, whole_page.offset, whole_page.size),
            (0, 0, 64)
        );
        Ok(())
    }

    #[test]
    fn stale_handles_cannot_free_reused_storage() -> Result<(), &'static str> {
        let mut arena = ArenaAllocator::new(32, 4);
        let old = arena.allocate(32).ok_or("initial allocation failed")?;
        assert!(arena.free(old));
        let current = arena.allocate(32).ok_or("replacement allocation failed")?;
        assert!(!arena.free(old));
        assert!(arena.free(current));
        assert_eq!(arena.stats().allocated_bytes, 0);
        Ok(())
    }
}
