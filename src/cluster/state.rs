use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{FailureReport, FailureTracker};

#[derive(Clone)]
pub struct ClusterState {
    failures: Arc<Mutex<FailureTracker>>,
}

impl ClusterState {
    pub fn new(quorum: usize, retention: Duration) -> Self {
        Self {
            failures: Arc::new(Mutex::new(FailureTracker::new(quorum, retention))),
        }
    }

    pub fn record_failure(&self, report: FailureReport) -> bool {
        self.failures.lock().unwrap().report(report)
    }

    pub fn failure_report_count(&self, target_id: &str, epoch: u64) -> usize {
        self.failures.lock().unwrap().report_count(target_id, epoch)
    }
}
