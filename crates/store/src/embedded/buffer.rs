use crossbeam::queue::SegQueue;
use std::sync::atomic::{AtomicUsize, Ordering};
use stomatopod_core::domain::event::Event;

/// Lock-free concurrent event buffer backed by a `crossbeam::SegQueue`.
/// The Parquet writer drains this; the ingest path pushes to it.
pub struct EventBuffer {
    inner: SegQueue<Event>,
    len: AtomicUsize,
    capacity: usize,
}

impl EventBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: SegQueue::new(),
            len: AtomicUsize::new(0),
            capacity,
        }
    }

    pub fn push(&self, event: Event) {
        self.inner.push(event);
        self.len.fetch_add(1, Ordering::Relaxed);
    }

    pub fn push_batch(&self, events: Vec<Event>) {
        let n = events.len();
        for e in events {
            self.inner.push(e);
        }
        self.len.fetch_add(n, Ordering::Relaxed);
    }

    /// Drain up to `max` events from the buffer.
    pub fn drain(&self, max: usize) -> Vec<Event> {
        let mut out = Vec::with_capacity(max);
        while out.len() < max {
            match self.inner.pop() {
                Some(e) => out.push(e),
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
