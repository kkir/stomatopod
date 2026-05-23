use std::collections::VecDeque;

use parking_lot::Mutex;

/// Why the local enforcement layer decided to trip.
#[derive(Debug, Clone, PartialEq)]
pub enum Trip {
    Repetition { count: u32, args_hash: String },
    TokenVelocity { tokens_per_sec: f64 },
    CostThreshold { usd: f64 },
}

/// Configurable thresholds. Mirrors `core::domain::policy::Policy` but
/// without the persistence-shape fields.
#[derive(Debug, Clone, Copy)]
pub struct PolicyConfig {
    pub repetition_max: Option<u32>,
    pub velocity_max_tps: Option<f64>,
    pub cost_cap_usd: Option<f64>,
    /// Rolling window for repetition detection.
    pub repetition_window: usize,
    /// Velocity window in milliseconds.
    pub velocity_window_ms: u64,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            repetition_max: Some(5),
            velocity_max_tps: Some(2000.0),
            cost_cap_usd: Some(10.0),
            repetition_window: 32,
            velocity_window_ms: 60_000,
        }
    }
}

/// Stateful per-process detector. Cheap to clone — internal state is in
/// an `Arc<Mutex<...>>` so the proxy can call it from multiple handlers.
pub struct PolicyEngine {
    cfg: PolicyConfig,
    state: Mutex<State>,
}

struct State {
    /// Sliding window of `tool_input_hash` values, newest at the back.
    recent_hashes: VecDeque<String>,
    /// (timestamp_ms, output_tokens) sliding window for velocity.
    velocity_samples: VecDeque<(u64, u32)>,
    /// Running cost per agent_session_id.
    session_cost: std::collections::HashMap<String, f64>,
}

impl PolicyEngine {
    pub fn new(cfg: PolicyConfig) -> Self {
        Self {
            cfg,
            state: Mutex::new(State {
                recent_hashes: VecDeque::new(),
                velocity_samples: VecDeque::new(),
                session_cost: std::collections::HashMap::new(),
            }),
        }
    }

    /// Record a tool call. Returns Some(Trip::Repetition) when the
    /// configured threshold is exceeded.
    pub fn observe_tool_call(&self, args_hash: &str) -> Option<Trip> {
        let mut s = self.state.lock();
        if s.recent_hashes.len() >= self.cfg.repetition_window {
            s.recent_hashes.pop_front();
        }
        s.recent_hashes.push_back(args_hash.to_string());
        if let Some(max) = self.cfg.repetition_max {
            let count = s.recent_hashes.iter().filter(|h| *h == args_hash).count() as u32;
            if count >= max {
                return Some(Trip::Repetition {
                    count,
                    args_hash: args_hash.to_string(),
                });
            }
        }
        None
    }

    /// Record a finished request. Returns Some(Trip::TokenVelocity)
    /// when output token rate exceeds the threshold over the window,
    /// or Some(Trip::CostThreshold) when this session has burned past
    /// the cap.
    pub fn observe_request(
        &self,
        session_id: &str,
        output_tokens: u32,
        cost_usd: f64,
        now_ms: u64,
    ) -> Option<Trip> {
        let mut s = self.state.lock();

        // Velocity: drop samples outside the window, then sum tokens.
        let cutoff = now_ms.saturating_sub(self.cfg.velocity_window_ms);
        while let Some(&(ts, _)) = s.velocity_samples.front() {
            if ts < cutoff {
                s.velocity_samples.pop_front();
            } else {
                break;
            }
        }
        s.velocity_samples.push_back((now_ms, output_tokens));
        if let Some(max_tps) = self.cfg.velocity_max_tps {
            // Need at least two distinct timestamps before talking about
            // a rate — a single sample is just a point.
            if s.velocity_samples.len() >= 2 {
                let total: u64 = s.velocity_samples.iter().map(|(_, t)| *t as u64).sum();
                let first_ts = s
                    .velocity_samples
                    .front()
                    .map(|(ts, _)| *ts)
                    .unwrap_or(now_ms);
                let span_ms = now_ms.saturating_sub(first_ts).max(1);
                let tps = (total as f64 * 1000.0) / span_ms as f64;
                if tps > max_tps {
                    return Some(Trip::TokenVelocity {
                        tokens_per_sec: tps,
                    });
                }
            }
        }

        // Cost cap per session. Bounded so a long-lived sidecar that sees
        // millions of distinct session ids doesn't grow without limit —
        // when we exceed `MAX_SESSIONS` we drop all session-cost state
        // (callers see this as a one-time reset of the cap window).
        const MAX_SESSIONS: usize = 10_000;
        if s.session_cost.len() >= MAX_SESSIONS && !s.session_cost.contains_key(session_id) {
            s.session_cost.clear();
        }
        let entry = s.session_cost.entry(session_id.to_string()).or_insert(0.0);
        *entry += cost_usd;
        if let Some(cap) = self.cfg.cost_cap_usd {
            if *entry > cap {
                return Some(Trip::CostThreshold { usd: *entry });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repetition_trips_after_threshold() {
        let pe = PolicyEngine::new(PolicyConfig {
            repetition_max: Some(3),
            ..PolicyConfig::default()
        });
        assert!(pe.observe_tool_call("h1").is_none());
        assert!(pe.observe_tool_call("h2").is_none());
        assert!(pe.observe_tool_call("h1").is_none());
        let trip = pe.observe_tool_call("h1").expect("should trip on 3rd h1");
        match trip {
            Trip::Repetition { count, args_hash } => {
                assert_eq!(count, 3);
                assert_eq!(args_hash, "h1");
            }
            _ => panic!("wrong trip kind"),
        }
    }

    #[test]
    fn cost_cap_trips() {
        let pe = PolicyEngine::new(PolicyConfig {
            cost_cap_usd: Some(1.0),
            velocity_max_tps: None,
            ..PolicyConfig::default()
        });
        assert!(pe.observe_request("s1", 100, 0.5, 0).is_none());
        let trip = pe.observe_request("s1", 100, 0.6, 1000).unwrap();
        assert!(matches!(trip, Trip::CostThreshold { .. }));
    }

    #[test]
    fn velocity_trips_over_threshold() {
        let pe = PolicyEngine::new(PolicyConfig {
            velocity_max_tps: Some(10.0),
            cost_cap_usd: None,
            velocity_window_ms: 60_000,
            ..PolicyConfig::default()
        });
        // 1000 tokens at t=0
        assert!(pe.observe_request("s", 1000, 0.0, 0).is_none());
        // Another 1000 tokens at t=100ms — that's 2000 tokens / 0.1s = 20k tps.
        let trip = pe.observe_request("s", 1000, 0.0, 100).unwrap();
        assert!(matches!(trip, Trip::TokenVelocity { .. }));
    }
}
