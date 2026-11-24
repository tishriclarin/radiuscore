//! Per-NAS and per-session admission quotas for authenticated request contexts.
use std::collections::HashMap;
use std::num::NonZeroU32;

pub type NasId = u32;
pub type SessionKey = [u8; 32];

#[derive(Clone, Copy)]
pub struct Rate {
    pub burst: NonZeroU32,
    pub per_second: NonZeroU32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Allowed,
    Unauthenticated,
    NasLimited,
    SessionLimited,
    Capacity,
}

struct Bucket {
    milli_tokens: u64,
    sampled_at: u64,
}

impl Bucket {
    fn full(rate: Rate, now_ms: u64) -> Self {
        Self {
            milli_tokens: u64::from(rate.burst.get()) * 1000,
            sampled_at: now_ms,
        }
    }

    fn take(&mut self, rate: Rate, now_ms: u64) -> bool {
        let elapsed = now_ms.saturating_sub(self.sampled_at);
        self.sampled_at = self.sampled_at.max(now_ms);
        self.milli_tokens = self
            .milli_tokens
            .saturating_add(elapsed.saturating_mul(u64::from(rate.per_second.get())))
            .min(u64::from(rate.burst.get()) * 1000);
        if self.milli_tokens < 1000 {
            return false;
        }
        self.milli_tokens -= 1000;
        true
    }
}

pub struct Limiter {
    nas_rate: Rate,
    session_rate: Rate,
    nas: HashMap<NasId, Bucket>,
    sessions: HashMap<(NasId, SessionKey), Bucket>,
    max_sessions: usize,
}

impl Limiter {
    pub fn new(
        authorized_nas: impl IntoIterator<Item = NasId>,
        nas_rate: Rate,
        session_rate: Rate,
        max_sessions: usize,
        now_ms: u64,
    ) -> Self {
        Self {
            nas_rate,
            session_rate,
            max_sessions,
            nas: authorized_nas
                .into_iter()
                .map(|id| (id, Bucket::full(nas_rate, now_ms)))
                .collect(),
            sessions: HashMap::new(),
        }
    }

    /// Call after verifying the peer. Unverified requests pass None and never allocate state.
    /// Pass a stable, scoped session key, if known, rather than the raw packet attribute.
    pub fn check(
        &mut self,
        authenticated_nas: Option<NasId>,
        session: Option<SessionKey>,
        now_ms: u64,
    ) -> Decision {
        let Some(id) = authenticated_nas else {
            return Decision::Unauthenticated;
        };
        let Some(nas) = self.nas.get_mut(&id) else {
            return Decision::Unauthenticated;
        };
        if !nas.take(self.nas_rate, now_ms) {
            return Decision::NasLimited;
        }
        let Some(key) = session else {
            return Decision::Allowed;
        };
        let full = self.sessions.len() >= self.max_sessions;
        let bucket = match self.sessions.entry((id, key)) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(_) if full => return Decision::Capacity,
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Bucket::full(self.session_rate, now_ms))
            }
        };
        if bucket.take(self.session_rate, now_ms) {
            Decision::Allowed
        } else {
            Decision::SessionLimited
        }
    }

    /// Run from a maintenance path. Only expire buckets that would have fully refilled.
    pub fn prune_idle(&mut self, now_ms: u64) {
        let refill_ms = (u64::from(self.session_rate.burst.get()) * 1000)
            .div_ceil(u64::from(self.session_rate.per_second.get()));
        self.sessions
            .retain(|_, bucket| now_ms.saturating_sub(bucket.sampled_at) < refill_ms);
    }
}
