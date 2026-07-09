use crossbeam::queue::SegQueue;
use std::sync::atomic::{AtomicUsize, Ordering};
use stomatopod_core::domain::event::Event;

/// Lock-free concurrent ingest buffer backed by a `crossbeam::SegQueue`.
///
/// Used by the Parquet writer (drains via `drain`) and the ingest path
/// (pushes via `push` / `push_batch` / `try_push_batch`).
pub struct Buffer<T> {
    inner: SegQueue<T>,
    len: AtomicUsize,
    capacity: usize,
}

impl<T> Buffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: SegQueue::new(),
            len: AtomicUsize::new(0),
            capacity,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn push(&self, item: T) {
        self.inner.push(item);
        self.len.fetch_add(1, Ordering::Relaxed);
    }

    /// Unbounded push used by WAL replay (startup may exceed capacity).
    pub fn push_batch(&self, items: Vec<T>) {
        let n = items.len();
        for item in items {
            self.inner.push(item);
        }
        self.len.fetch_add(n, Ordering::Relaxed);
    }

    /// Push a batch only if it fits within capacity. On rejection returns the
    /// items unpushed so the caller can flush and retry.
    pub fn try_push_batch(&self, items: Vec<T>) -> Result<(), Vec<T>> {
        let n = items.len();
        let current = self.len.load(Ordering::Relaxed);
        if current.saturating_add(n) > self.capacity {
            return Err(items);
        }
        for item in items {
            self.inner.push(item);
        }
        self.len.fetch_add(n, Ordering::Relaxed);
        Ok(())
    }

    /// Drain up to `max` items from the buffer.
    pub fn drain(&self, max: usize) -> Vec<T> {
        let mut out = Vec::with_capacity(max);
        while out.len() < max {
            match self.inner.pop() {
                Some(item) => out.push(item),
                None => break,
            }
        }
        let n = out.len();
        if n > 0 {
            self.len.fetch_sub(n, Ordering::Relaxed);
        }
        out
    }

    pub fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_above_threshold(&self) -> bool {
        self.len() >= (self.capacity * 9) / 10
    }
}

pub type EventBuffer = Buffer<Event>;
