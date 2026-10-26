//! Successful authentication is not an unlimited work budget.
use crate::devices::Capability;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

type Records = HashMap<(String, u8), (Instant, u32)>;
#[derive(Default)]
pub struct CommandBudget {
    records: Mutex<Records>,
}
impl CommandBudget {
    pub fn admit(&self, device: &str, capability: Capability) -> bool {
        self.admit_at(device, capability, Instant::now())
    }
    fn admit_at(&self, device: &str, capability: Capability, now: Instant) -> bool {
        let (class, maximum) = match capability {
            Capability::Input | Capability::Media => (0, 120),
            Capability::Power => (1, 1),
            Capability::Wake => (2, 4),
            Capability::Status => (3, 20),
        };
        let mut records = self.records.lock().unwrap_or_else(|p| p.into_inner());
        records.retain(|_, (at, _)| now.duration_since(*at) < Duration::from_secs(1));
        let key = (device.to_owned(), class);
        if records.len() >= 512 && !records.contains_key(&key) {
            return false;
        }
        let (_, count) = records.entry(key).or_insert((now, 0));
        if *count >= maximum {
            return false;
        }
        *count += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn power_and_wake_budgets_are_separate_bounded_and_expire() {
        let budget = CommandBudget::default();
        let now = Instant::now();
        assert!(budget.admit_at("a", Capability::Power, now));
        assert!(!budget.admit_at("a", Capability::Power, now));
        assert!(budget.admit_at("b", Capability::Power, now));
        for _ in 0..4 {
            assert!(budget.admit_at("a", Capability::Wake, now));
        }
        assert!(!budget.admit_at("a", Capability::Wake, now));
        assert!(budget.admit_at("a", Capability::Power, now + Duration::from_secs(2)));
    }
}
