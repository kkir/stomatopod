use crossbeam::queue::SegQueue;
use std::sync::atomic::{AtomicUsize, Ordering};
use stomatopod_core::domain::agent_span::AgentSpan;

/// Lock-free concurrent span buffer. Mirrors `EventBuffer` but holds
/// `AgentSpan`s. Spans are higher-frequency than pageviews, so it gets
/// its own buffer + WAL to avoid serializing the analytics hot path.
pub struct SpanBuffer {
    inner: SegQueue<AgentSpan>,
    len: AtomicUsize,
    capacity: usize,
}

impl SpanBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: SegQueue::new(),
            len: AtomicUsize::new(0),
            capacity,
        }
    }

    pub fn push_batch(&self, spans: Vec<AgentSpan>) {
        let n = spans.len();
        for s in spans {
            self.inner.push(s);
        }
        self.len.fetch_add(n, Ordering::Relaxed);
    }

    pub fn drain(&self, max: usize) -> Vec<AgentSpan> {
        let mut out = Vec::with_capacity(max);
        while out.len() < max {
            match self.inner.pop() {
                Some(s) => out.push(s),
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
