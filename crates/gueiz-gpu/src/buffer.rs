use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::Gueiz2DError;
use crate::error::Gueiz2DError::{
    DevicePollError, FrameRegionExhaustedError, HeapExhaustedError, InvalidAllocationError,
};

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub struct Allocation {
    offset: u64,
    size: u64,
    allocation_id: u64,
}

impl Allocation {
    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn range(&self) -> Range<u64> {
        self.offset..self.offset + self.size
    }

    pub fn is_frame(&self) -> bool {
        self.allocation_id == FRAME_ALLOCATION_ID
    }
}

#[derive(Clone, Copy)]
#[derive(Debug)]
struct Block {
    offset: u64,
    size: u64,
    state: BlockState,
}

impl Block {
    fn end(&self) -> u64 {
        self.offset + self.size
    }

    fn is_free(&self) -> bool {
        matches!(self.state, BlockState::Free)
    }
}

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
enum BlockState {
    Free,
    Used(u64),
}

#[derive(Debug)]
pub struct Heap {
    size: u64,
    alignment: u64,
    blocks: Vec<Block>,
    used: u64,
}

impl Heap {
    pub fn new(size: u64) -> Self {
        Self::with_alignment(size, 4)
    }

    pub fn with_alignment(size: u64, alignment: u64) -> Self {
        assert!(
            alignment.is_power_of_two(),
            "alignment must be a power of two, got {alignment}",
        );

        let mut blocks = Vec::new();
        if size > 0 {
            blocks.push(Block {
                offset: 0,
                size,
                state: BlockState::Free,
            });
        }

        Self {
            size,
            alignment,
            blocks,
            used: 0,
        }
    }

    pub fn allocate(&mut self, size: u64) -> Result<Allocation, Gueiz2DError> {
        self.allocate_aligned(size, self.alignment)
    }

    pub fn allocate_aligned(
        &mut self,
        size: u64,
        alignment: u64,
    ) -> Result<Allocation, Gueiz2DError> {
        assert!(
            alignment.is_power_of_two(),
            "alignment must be a power of two, got {alignment}",
        );

        let alignment = alignment.max(self.alignment);
        let padded_size = align_up(size.max(1), alignment);

        let Some((index, aligned_offset)) = self.find_best_fit(padded_size, alignment) else {
            return Err(HeapExhaustedError {
                requested: padded_size,
                largest_free_block: self.largest_free_block(),
            });
        };

        let allocation_id = next_allocation_id();

        self.split(index, aligned_offset, padded_size, allocation_id);
        self.used += padded_size;

        Ok(Allocation {
            offset: aligned_offset,
            size,
            allocation_id,
        })
    }

    pub fn deallocate(&mut self, allocation: Allocation) -> Result<(), Gueiz2DError> {
        if allocation.is_frame() {
            return Err(InvalidAllocationError);
        }

        let index = self
            .blocks
            .binary_search_by_key(&allocation.offset, |block| block.offset)
            .map_err(|_| InvalidAllocationError)?;

        if self.blocks[index].state != BlockState::Used(allocation.allocation_id) {
            return Err(InvalidAllocationError);
        }

        self.used -= self.blocks[index].size;
        self.blocks[index].state = BlockState::Free;

        self.coalesce_with_next(index);
        if index > 0 {
            self.coalesce_with_next(index - 1);
        }

        Ok(())
    }

    pub fn reset(&mut self) {
        self.blocks.clear();
        if self.size > 0 {
            self.blocks.push(Block {
                offset: 0,
                size: self.size,
                state: BlockState::Free,
            });
        }
        self.used = 0;
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn available(&self) -> u64 {
        self.size - self.used
    }

    pub fn largest_free_block(&self) -> u64 {
        self.blocks
            .iter()
            .filter(|block| block.is_free())
            .map(|block| block.size)
            .max()
            .unwrap_or(0)
    }

    pub fn fragmentation(&self) -> f32 {
        let available = self.available();
        if available == 0 {
            return 0.0;
        }

        1.0 - (self.largest_free_block() as f32 / available as f32)
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    fn find_best_fit(&self, padded_size: u64, alignment: u64) -> Option<(usize, u64)> {
        let mut best: Option<(usize, u64, u64)> = None;

        for (index, block) in self.blocks.iter().enumerate() {
            if !block.is_free() {
                continue;
            }

            let aligned_offset = align_up(block.offset, alignment);
            let Some(usable) = block.end().checked_sub(aligned_offset) else {
                continue;
            };
            if usable < padded_size {
                continue;
            }

            let waste = block.size - padded_size;
            if best.is_none_or(|(_, _, best_waste)| waste < best_waste) {
                best = Some((index, aligned_offset, waste));
            }
        }

        best.map(|(index, aligned_offset, _)| (index, aligned_offset))
    }

    fn split(&mut self, index: usize, aligned_offset: u64, padded_size: u64, allocation_id: u64) {
        let block = self.blocks[index];
        debug_assert!(block.is_free());

        let head_padding = aligned_offset - block.offset;
        let tail_size = block.end() - (aligned_offset + padded_size);

        self.blocks[index] = Block {
            offset: aligned_offset,
            size: padded_size,
            state: BlockState::Used(allocation_id),
        };

        if tail_size > 0 {
            self.blocks.insert(
                index + 1,
                Block {
                    offset: aligned_offset + padded_size,
                    size: tail_size,
                    state: BlockState::Free,
                },
            );
        }

        if head_padding > 0 {
            self.blocks.insert(
                index,
                Block {
                    offset: block.offset,
                    size: head_padding,
                    state: BlockState::Free,
                },
            );
        }
    }

    fn coalesce_with_next(&mut self, index: usize) {
        let Some(next) = self.blocks.get(index + 1).copied() else {
            return;
        };

        if !self.blocks[index].is_free() || !next.is_free() {
            return;
        }

        self.blocks[index].size += next.size;
        self.blocks.remove(index + 1);
    }
}

static NEXT_ALLOCATION_ID: AtomicU64 = AtomicU64::new(1);

fn next_allocation_id() -> u64 {
    NEXT_ALLOCATION_ID.fetch_add(1, Ordering::Relaxed)
}

fn align_up(value: u64, alignment: u64) -> u64 {
    (value + alignment - 1) & !(alignment - 1)
}

const FRAME_ALLOCATION_ID: u64 = 0;

fn align_down(value: u64, alignment: u64) -> u64 {
    value & !(alignment - 1)
}

#[derive(Debug, Default)]
struct FrameSlice {
    head: u64,
    submission_index: Option<wgpu::SubmissionIndex>,
}

#[derive(Debug)]
pub struct FrameRegion {
    offset: u64,
    slice_size: u64,
    alignment: u64,
    slices: Vec<FrameSlice>,
    current: usize,
}

impl FrameRegion {
    fn new(offset: u64, slice_size: u64, slice_count: u32, alignment: u64) -> Self {
        let slice_count = if slice_size == 0 { 0 } else { slice_count as usize };

        let mut slices = Vec::with_capacity(slice_count);
        slices.resize_with(slice_count, FrameSlice::default);

        Self {
            offset,
            slice_size,
            alignment,
            slices,
            current: slice_count.saturating_sub(1),
        }
    }

    fn allocate(&mut self, size: u64, alignment: u64) -> Result<Allocation, Gueiz2DError> {
        assert!(
            alignment.is_power_of_two(),
            "alignment must be a power of two, got {alignment}",
        );

        let alignment = alignment.max(self.alignment);
        let padded_size = align_up(size.max(1), alignment);

        let slice_offset = self.offset + self.current as u64 * self.slice_size;
        let slice_size = self.slice_size;

        let Some(slice) = self.slices.get_mut(self.current) else {
            return Err(FrameRegionExhaustedError {
                requested: padded_size,
                available: 0,
            });
        };

        let aligned_offset = align_up(slice_offset + slice.head, alignment);
        let slice_end = slice_offset + slice_size;

        if aligned_offset + padded_size > slice_end {
            return Err(FrameRegionExhaustedError {
                requested: padded_size,
                available: slice_end - (slice_offset + slice.head),
            });
        }

        slice.head = aligned_offset + padded_size - slice_offset;

        Ok(Allocation {
            offset: aligned_offset,
            size,
            allocation_id: FRAME_ALLOCATION_ID,
        })
    }

    fn advance(&mut self) -> Option<wgpu::SubmissionIndex> {
        if self.slices.is_empty() {
            return None;
        }

        self.current = (self.current + 1) % self.slices.len();
        self.slices[self.current].submission_index.take()
    }

    fn reset_current(&mut self) {
        if let Some(slice) = self.slices.get_mut(self.current) {
            slice.head = 0;
        }
    }

    fn set_submission_index(&mut self, submission_index: wgpu::SubmissionIndex) {
        if let Some(slice) = self.slices.get_mut(self.current) {
            slice.submission_index = Some(submission_index);
        }
    }

    pub fn size(&self) -> u64 {
        self.slice_size * self.slices.len() as u64
    }

    pub fn slice_size(&self) -> u64 {
        self.slice_size
    }

    pub fn slice_count(&self) -> usize {
        self.slices.len()
    }

    pub fn current_slice(&self) -> usize {
        self.current
    }

    pub fn used(&self) -> u64 {
        self.slices
            .get(self.current)
            .map(|slice| slice.head)
            .unwrap_or(0)
    }

    pub fn available(&self) -> u64 {
        self.slice_size - self.used()
    }
}

pub struct BufferHeapDescriptor<'a> {
    pub label: Option<&'a str>,
    pub size: u64,
    pub frame_size: u64,
    pub frames_in_flight: u32,
    pub usage: wgpu::BufferUsages,
    pub alignment: u64,
}

impl Default for BufferHeapDescriptor<'_> {
    fn default() -> Self {
        Self {
            label: None,
            size: 0,
            frame_size: 0,
            frames_in_flight: 2,
            usage: wgpu::BufferUsages::empty(),
            alignment: 4,
        }
    }
}

pub struct BufferHeap {
    buffer: wgpu::Buffer,
    usage: wgpu::BufferUsages,
    size: u64,
    heap: Heap,
    frame_region: FrameRegion,
}

impl BufferHeap {
    pub fn new(device: &wgpu::Device, descriptor: &BufferHeapDescriptor<'_>) -> Self {
        assert!(
            descriptor.alignment.is_power_of_two(),
            "alignment must be a power of two, got {}",
            descriptor.alignment,
        );
        assert!(
            descriptor.frames_in_flight > 0,
            "frames_in_flight must be at least 1",
        );

        let alignment = descriptor.alignment;
        let size = align_up(descriptor.size, alignment);
        let frames_in_flight = descriptor.frames_in_flight as u64;

        let slice_size = align_down(descriptor.frame_size / frames_in_flight, alignment);
        let frame_region_size = slice_size * frames_in_flight;

        assert!(
            frame_region_size <= size,
            "frame region ({frame_region_size} bytes) does not fit in the buffer ({size} bytes)",
        );

        let frame_floor = size - frame_region_size;

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: descriptor.label,
            size,
            usage: descriptor.usage,
            mapped_at_creation: false,
        });

        Self {
            buffer,
            usage: descriptor.usage,
            size,
            heap: Heap::with_alignment(frame_floor, alignment),
            frame_region: FrameRegion::new(
                frame_floor,
                slice_size,
                descriptor.frames_in_flight,
                alignment,
            ),
        }
    }

    pub fn allocate(&mut self, size: u64) -> Result<Allocation, Gueiz2DError> {
        self.heap.allocate(size)
    }

    pub fn allocate_aligned(
        &mut self,
        size: u64,
        alignment: u64,
    ) -> Result<Allocation, Gueiz2DError> {
        self.heap.allocate_aligned(size, alignment)
    }

    pub fn allocate_frame(&mut self, size: u64) -> Result<Allocation, Gueiz2DError> {
        self.frame_region.allocate(size, self.frame_region.alignment)
    }

    pub fn allocate_frame_aligned(
        &mut self,
        size: u64,
        alignment: u64,
    ) -> Result<Allocation, Gueiz2DError> {
        self.frame_region.allocate(size, alignment)
    }

    pub fn deallocate(&mut self, allocation: Allocation) -> Result<(), Gueiz2DError> {
        self.heap.deallocate(allocation)
    }

    pub fn begin_frame(&mut self, device: &wgpu::Device) -> Result<(), Gueiz2DError> {
        if let Some(submission_index) = self.frame_region.advance() {
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission_index),
                    timeout: None,
                })
                .map_err(DevicePollError)?;
        }

        self.frame_region.reset_current();

        Ok(())
    }

    pub fn end_frame(&mut self, submission_index: wgpu::SubmissionIndex) {
        self.frame_region.set_submission_index(submission_index);
    }

    pub fn write(
        &self,
        queue: &wgpu::Queue,
        allocation: &Allocation,
        data: &[u8],
    ) -> Result<(), Gueiz2DError> {
        if data.len() as u64 > allocation.size() {
            return Err(InvalidAllocationError);
        }

        queue.write_buffer(&self.buffer, allocation.offset(), data);

        Ok(())
    }

    pub fn slice(&self, allocation: &Allocation) -> wgpu::BufferSlice<'_> {
        self.buffer.slice(allocation.range())
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    pub fn usage(&self) -> wgpu::BufferUsages {
        self.usage
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// 前方ヒープの全区画を解放する。形状を丸ごと積み直すときに使う。
    pub fn reset_heap(&mut self) {
        self.heap.reset();
    }

    pub fn heap(&self) -> &Heap {
        &self.heap
    }

    pub fn frame_region(&self) -> &FrameRegion {
        &self.frame_region
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_invariants(heap: &Heap) {
        let mut expected_offset = 0;
        let mut previous_was_free = false;

        for block in &heap.blocks {
            assert_eq!(block.offset, expected_offset, "blocks must be contiguous");
            assert!(block.size > 0, "zero-sized blocks must not exist");
            assert!(
                !(previous_was_free && block.is_free()),
                "adjacent free blocks must be coalesced",
            );

            expected_offset = block.end();
            previous_was_free = block.is_free();
        }

        assert_eq!(expected_offset, heap.size(), "blocks must cover the heap");
    }

    #[test]
    fn allocates_from_the_front() {
        let mut heap = Heap::new(1024);

        let first = heap.allocate(100).unwrap();
        let second = heap.allocate(100).unwrap();

        assert_eq!(first.offset(), 0);
        assert_eq!(second.offset(), 100);
        assert_eq!(heap.used(), 200);
        assert_invariants(&heap);
    }

    #[test]
    fn rounds_size_up_to_the_alignment() {
        let mut heap = Heap::new(1024);

        let first = heap.allocate(1).unwrap();
        let second = heap.allocate(1).unwrap();

        assert_eq!(first.size(), 1, "requested size is reported as-is");
        assert_eq!(second.offset(), 4, "but the next block starts aligned");
        assert_eq!(heap.used(), 8);
        assert_invariants(&heap);
    }

    #[test]
    fn coalesces_with_the_next_block() {
        let mut heap = Heap::new(1024);

        let first = heap.allocate(256).unwrap();
        let second = heap.allocate(256).unwrap();

        heap.deallocate(second).unwrap();
        heap.deallocate(first).unwrap();

        assert_eq!(heap.block_count(), 1, "everything merged back into one block");
        assert_eq!(heap.largest_free_block(), 1024);
        assert_invariants(&heap);
    }

    #[test]
    fn coalesces_with_both_neighbours() {
        let mut heap = Heap::new(1024);

        let first = heap.allocate(256).unwrap();
        let middle = heap.allocate(256).unwrap();
        let last = heap.allocate(256).unwrap();

        heap.deallocate(first).unwrap();
        heap.deallocate(last).unwrap();
        assert_eq!(heap.block_count(), 3);

        heap.deallocate(middle).unwrap();
        assert_eq!(heap.block_count(), 1);
        assert_eq!(heap.used(), 0);
        assert_invariants(&heap);
    }

    #[test]
    fn reuses_a_freed_hole() {
        let mut heap = Heap::new(1024);

        let first = heap.allocate(256).unwrap();
        let _second = heap.allocate(256).unwrap();
        heap.deallocate(first).unwrap();

        let reused = heap.allocate(256).unwrap();

        assert_eq!(reused.offset(), 0, "the hole at the front is reused");
        assert_invariants(&heap);
    }

    #[test]
    fn picks_the_smallest_fitting_hole() {
        let mut heap = Heap::new(1024);

        let big = heap.allocate(256).unwrap();
        let _keep_big = heap.allocate(16).unwrap();
        let small = heap.allocate(64).unwrap();
        let _keep_small = heap.allocate(16).unwrap();

        heap.deallocate(big).unwrap();
        heap.deallocate(small).unwrap();

        let allocation = heap.allocate(64).unwrap();

        assert_eq!(allocation.offset(), small.offset());
        assert_invariants(&heap);
    }

    #[test]
    fn honours_a_stricter_alignment() {
        let mut heap = Heap::new(1024);

        let _head = heap.allocate(4).unwrap();
        let aligned = heap.allocate_aligned(16, 256).unwrap();

        assert_eq!(aligned.offset(), 256);
        assert_invariants(&heap);
    }

    #[test]
    fn reuses_the_padding_left_by_alignment() {
        let mut heap = Heap::new(1024);

        let _head = heap.allocate(4).unwrap();
        let _aligned = heap.allocate_aligned(16, 256).unwrap();

        let padding = heap.allocate(4).unwrap();

        assert_eq!(padding.offset(), 4);
        assert_invariants(&heap);
    }

    #[test]
    fn reports_exhaustion_with_the_largest_hole() {
        let mut heap = Heap::new(1024);

        let first = heap.allocate(512).unwrap();
        let _second = heap.allocate(256).unwrap();
        heap.deallocate(first).unwrap();

        let error = heap.allocate(768).unwrap_err();

        assert!(matches!(
            error,
            HeapExhaustedError { requested: 768, largest_free_block: 512 },
        ));
        assert_eq!(heap.available(), 768);
        assert_invariants(&heap);
    }

    #[test]
    fn rejects_a_double_free() {
        let mut heap = Heap::new(1024);

        let allocation = heap.allocate(256).unwrap();
        heap.deallocate(allocation).unwrap();

        assert!(matches!(
            heap.deallocate(allocation),
            Err(InvalidAllocationError),
        ));
        assert_invariants(&heap);
    }

    #[test]
    fn rejects_an_allocation_from_another_heap() {
        let mut heap = Heap::new(1024);
        let mut other = Heap::new(1024);

        let foreign = other.allocate(256).unwrap();
        let _mine = heap.allocate(256).unwrap();

        assert!(matches!(
            heap.deallocate(foreign),
            Err(InvalidAllocationError),
        ));
    }

    #[test]
    fn reset_invalidates_outstanding_allocations() {
        let mut heap = Heap::new(1024);

        let allocation = heap.allocate(256).unwrap();
        heap.reset();

        assert_eq!(heap.used(), 0);
        assert_eq!(heap.block_count(), 1);
        assert!(matches!(
            heap.deallocate(allocation),
            Err(InvalidAllocationError),
        ));
    }

    #[test]
    fn measures_fragmentation() {
        let mut heap = Heap::new(1024);

        assert_eq!(heap.fragmentation(), 0.0, "one big hole is not fragmented");

        let first = heap.allocate(256).unwrap();
        let _second = heap.allocate(256).unwrap();
        let third = heap.allocate(256).unwrap();
        let _fourth = heap.allocate(256).unwrap();

        heap.deallocate(first).unwrap();
        heap.deallocate(third).unwrap();

        assert_eq!(heap.available(), 512);
        assert_eq!(heap.largest_free_block(), 256);
        assert_eq!(heap.fragmentation(), 0.5);
        assert_invariants(&heap);
    }

    #[test]
    fn survives_a_churn_of_mixed_sizes() {
        let mut heap = Heap::new(64 * 1024);
        let mut allocations = Vec::new();

        for round in 0..64u64 {
            let size = (round * 37) % 500 + 1;
            allocations.push(heap.allocate(size).unwrap());

            if round % 3 == 0 && !allocations.is_empty() {
                let index = (round as usize * 7) % allocations.len();
                heap.deallocate(allocations.swap_remove(index)).unwrap();
            }

            assert_invariants(&heap);
        }

        for allocation in allocations {
            heap.deallocate(allocation).unwrap();
        }

        assert_eq!(heap.used(), 0);
        assert_eq!(heap.block_count(), 1, "a full drain must leave one free block");
    }

    #[test]
    fn an_empty_heap_allocates_nothing() {
        let mut heap = Heap::new(0);

        assert_eq!(heap.block_count(), 0);
        assert!(heap.allocate(1).is_err());
        assert_invariants(&heap);
    }

    fn frame_region(frame_floor: u64, slice_size: u64, slice_count: u32) -> FrameRegion {
        let mut region = FrameRegion::new(frame_floor, slice_size, slice_count, 4);
        // 最初のフレームに入る。
        region.advance();
        region.reset_current();
        region
    }

    #[test]
    fn frame_allocations_bump_within_a_slice() {
        let mut region = frame_region(1024, 256, 2);

        let first = region.allocate(16, 4).unwrap();
        let second = region.allocate(16, 4).unwrap();

        assert_eq!(first.offset(), 1024, "the region starts at the frame floor");
        assert_eq!(second.offset(), 1040);
        assert_eq!(region.used(), 32);
        assert_eq!(region.available(), 224);
    }

    #[test]
    fn frame_allocations_are_tagged_as_frame() {
        let mut region = frame_region(1024, 256, 2);

        let allocation = region.allocate(16, 4).unwrap();

        assert!(allocation.is_frame());
    }

    #[test]
    fn frame_allocations_cannot_be_returned_to_the_heap() {
        let mut heap = Heap::new(1024);
        let mut region = frame_region(1024, 256, 2);

        let allocation = region.allocate(16, 4).unwrap();

        assert!(matches!(
            heap.deallocate(allocation),
            Err(InvalidAllocationError),
        ));
    }

    #[test]
    fn frame_slices_rotate_and_reset() {
        let mut region = frame_region(1024, 256, 2);

        region.allocate(64, 4).unwrap();
        assert_eq!(region.current_slice(), 0);
        assert_eq!(region.used(), 64);

        region.advance();
        region.reset_current();
        assert_eq!(region.current_slice(), 1);
        assert_eq!(region.used(), 0);

        let second_frame = region.allocate(64, 4).unwrap();
        assert_eq!(second_frame.offset(), 1024 + 256, "slice 1 starts one slice in");

        region.advance();
        region.reset_current();
        assert_eq!(region.current_slice(), 0);

        let third_frame = region.allocate(64, 4).unwrap();
        assert_eq!(third_frame.offset(), 1024);
    }

    #[test]
    fn frame_slices_do_not_overlap() {
        let mut region = frame_region(1024, 256, 3);
        let mut offsets = Vec::new();

        for _ in 0..3 {
            offsets.push(region.allocate(256, 4).unwrap());
            region.advance();
            region.reset_current();
        }

        for (index, allocation) in offsets.iter().enumerate() {
            assert_eq!(allocation.offset(), 1024 + index as u64 * 256);
        }
    }

    #[test]
    fn frame_region_reports_exhaustion() {
        let mut region = frame_region(1024, 256, 2);

        region.allocate(200, 4).unwrap();
        let error = region.allocate(100, 4).unwrap_err();

        assert!(matches!(
            error,
            FrameRegionExhaustedError { requested: 100, available: 56 },
        ));
    }

    #[test]
    fn frame_region_honours_a_stricter_alignment() {
        let mut region = frame_region(1024, 1024, 1);

        let _head = region.allocate(4, 4).unwrap();
        let aligned = region.allocate(16, 256).unwrap();

        assert_eq!(aligned.offset(), 1024 + 256);
    }

    #[test]
    fn a_region_without_slices_allocates_nothing() {
        let mut region = FrameRegion::new(1024, 0, 2, 4);

        assert_eq!(region.slice_count(), 0);
        assert_eq!(region.size(), 0);
        assert!(region.advance().is_none());
        assert!(matches!(
            region.allocate(16, 4),
            Err(FrameRegionExhaustedError { available: 0, .. }),
        ));
    }

    #[test]
    fn the_layout_splits_the_buffer_without_overlap() {
        let size = 4096u64;
        let slice_size = align_down(1024 / 2, 4);
        let frame_region_size = slice_size * 2;
        let frame_floor = size - frame_region_size;

        let heap = Heap::with_alignment(frame_floor, 4);
        let region = FrameRegion::new(frame_floor, slice_size, 2, 4);

        assert_eq!(heap.size(), 3072, "the heap owns everything below the floor");
        assert_eq!(region.size(), 1024);
        assert_eq!(heap.size() + region.size(), size, "the two regions tile the buffer");
    }
}
