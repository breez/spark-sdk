//! The records sspd is serving a call for.

use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};

#[derive(Default)]
pub struct InFlight(Mutex<HashSet<String>>);

impl InFlight {
    /// Marks `id` as being served until the caller drops the returned guard.
    /// Returns `None` while another caller is serving `id`.
    pub fn start(&self, id: &str) -> Option<InFlightCall<'_>> {
        let mut running = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        running.insert(id.to_string()).then(|| InFlightCall {
            calls: self,
            id: id.to_string(),
        })
    }
}

pub struct InFlightCall<'a> {
    calls: &'a InFlight,
    id: String,
}

impl Drop for InFlightCall<'_> {
    fn drop(&mut self) {
        self.calls
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_is_running_until_its_guard_drops() {
        let calls = InFlight::default();
        let call = calls.start("claim").expect("nothing is running");
        assert!(calls.start("claim").is_none());
        assert!(calls.start("other").is_some());
        drop(call);
        assert!(calls.start("claim").is_some());
    }
}
