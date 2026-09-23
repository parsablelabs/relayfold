use crate::core::worker::{WorkerHeartbeatState, WorkerHostId, WorkerIdentity};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::debug;

const DEFAULT_WORKER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_MISSED_HEARTBEAT_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerHeartbeatPolicy {
    pub heartbeat_interval_ms: u64,
    pub missed_heartbeat_threshold: u32,
}

#[derive(Debug)]
struct WorkerState {
    identity: WorkerIdentity,
    heartbeat: WorkerHeartbeatState,
}

#[derive(Debug, Clone)]
pub struct WorkerRegistry {
    workers: Arc<RwLock<HashMap<String, WorkerState>>>,
    heartbeat_interval: Duration,
    missed_heartbeat_threshold: u32,
    next_host_selection: Arc<AtomicUsize>,
}

impl Default for WorkerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerRegistry {
    pub fn new() -> Self {
        Self::new_with_heartbeat_config(
            DEFAULT_WORKER_HEARTBEAT_INTERVAL,
            DEFAULT_MISSED_HEARTBEAT_THRESHOLD,
        )
    }

    fn new_with_heartbeat_config(
        heartbeat_interval: Duration,
        missed_heartbeat_threshold: u32,
    ) -> Self {
        Self {
            workers: Arc::new(RwLock::new(HashMap::new())),
            heartbeat_interval,
            missed_heartbeat_threshold,
            next_host_selection: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub async fn register_worker(&self, identity: WorkerIdentity) {
        self.tick_worker_heartbeat(identity).await;
    }

    pub async fn tick_worker_heartbeat(&self, identity: WorkerIdentity) {
        let worker_id = identity.worker_id.0.clone();
        let host_id = identity.host_id.0.clone();
        let now = epoch_ms();
        let heartbeat = self.heartbeat_state(identity.clone(), now);
        let mut workers = self.workers.write().await;
        match workers.get_mut(&worker_id) {
            Some(worker) => {
                worker.identity = identity;
                worker.heartbeat = heartbeat;
            }
            None => {
                workers.insert(
                    worker_id.clone(),
                    WorkerState {
                        identity,
                        heartbeat,
                    },
                );
            }
        }

        debug!(%worker_id, %host_id, "worker heartbeat joined or renewed registration");
    }

    pub async fn worker_count(&self) -> usize {
        self.workers.read().await.len()
    }

    pub fn heartbeat_policy(&self) -> WorkerHeartbeatPolicy {
        WorkerHeartbeatPolicy {
            heartbeat_interval_ms: self.heartbeat_interval.as_millis() as u64,
            missed_heartbeat_threshold: self.missed_heartbeat_threshold,
        }
    }

    pub async fn select_eligible_host(&self) -> Option<WorkerHostId> {
        self.update_worker_liveness().await;

        let eligible_hosts = self.eligible_hosts().await;
        if eligible_hosts.is_empty() {
            return None;
        }

        let index = self.next_host_selection.fetch_add(1, Ordering::Relaxed) % eligible_hosts.len();

        eligible_hosts.get(index).cloned()
    }

    pub async fn select_force_retry_host(
        &self,
        current_host: Option<&WorkerHostId>,
    ) -> Option<WorkerHostId> {
        self.update_worker_liveness().await;
        let eligible_hosts = self.eligible_hosts().await;

        if let Some(current_host) = current_host {
            if eligible_hosts.contains(current_host) {
                return Some(current_host.clone());
            }
        }

        eligible_hosts
            .into_iter()
            .find(|host_id| Some(host_id) != current_host)
    }

    pub async fn worker_identity_for_claim(
        &self,
        worker_id: &str,
    ) -> anyhow::Result<WorkerIdentity> {
        let workers = self.workers.read().await;
        let Some(worker) = workers.get(worker_id) else {
            anyhow::bail!("worker {worker_id} is not registered");
        };

        if worker.heartbeat.missed_heartbeat {
            anyhow::bail!("worker {worker_id} missed heartbeat");
        }

        Ok(worker.identity.clone())
    }

    pub async fn update_worker_liveness(&self) -> Vec<WorkerHostId> {
        let now = epoch_ms();
        let mut deregistered_hosts = Vec::new();
        let mut workers = self.workers.write().await;
        workers.retain(|worker_id, worker| {
            if worker.heartbeat.deregister_after_epoch_ms <= now {
                let host_id = worker.identity.host_id.clone();
                debug!(%worker_id, host_id = %host_id.0, "deregistering worker after missed heartbeat threshold");
                deregistered_hosts.push(host_id);
                return false;
            }

            worker.heartbeat.missed_heartbeat = worker.heartbeat.next_heartbeat_due_epoch_ms <= now;
            true
        });

        if deregistered_hosts.is_empty() {
            return vec![];
        }

        let remaining_hosts = workers
            .values()
            .map(|worker| worker.identity.host_id.clone())
            .collect::<HashSet<_>>();

        let mut lost_hosts = deregistered_hosts
            .into_iter()
            .filter(|host_id| !remaining_hosts.contains(host_id))
            .collect::<Vec<_>>();
        lost_hosts.sort_by(|left, right| left.0.cmp(&right.0));
        lost_hosts.dedup();
        lost_hosts
    }

    async fn eligible_hosts(&self) -> Vec<WorkerHostId> {
        let workers = self.workers.read().await;
        let mut host_ids = workers
            .values()
            .filter(|worker| !worker.heartbeat.missed_heartbeat)
            .map(|worker| worker.identity.host_id.clone())
            .collect::<Vec<_>>();
        host_ids.sort_by(|left, right| left.0.cmp(&right.0));
        host_ids.dedup();
        host_ids
    }

    fn heartbeat_state(&self, identity: WorkerIdentity, now_epoch_ms: u64) -> WorkerHeartbeatState {
        let interval_ms = self.heartbeat_interval.as_millis() as u64;
        let threshold = u64::from(self.missed_heartbeat_threshold.max(1));
        WorkerHeartbeatState {
            identity,
            last_heartbeat_at_epoch_ms: now_epoch_ms,
            next_heartbeat_due_epoch_ms: now_epoch_ms + interval_ms,
            deregister_after_epoch_ms: now_epoch_ms + (interval_ms * threshold),
            missed_heartbeat: false,
        }
    }
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::worker::{WorkerHostId, WorkerId};
    use tokio::time;

    fn test_registration(worker_id: &str) -> WorkerIdentity {
        test_registration_for_host(worker_id, "test-host")
    }

    fn test_registration_for_host(worker_id: &str, host_id: &str) -> WorkerIdentity {
        WorkerIdentity {
            worker_id: WorkerId::new(worker_id),
            host_id: WorkerHostId::new(host_id),
        }
    }

    fn heartbeat_test_registry() -> WorkerRegistry {
        WorkerRegistry::new_with_heartbeat_config(Duration::from_millis(10), 3)
    }

    #[tokio::test]
    async fn registration_preserves_worker_identity_separately_from_host_identity() {
        let registry = WorkerRegistry::new();
        registry
            .register_worker(test_registration("worker-1"))
            .await;
        registry
            .register_worker(test_registration("worker-2"))
            .await;

        let worker_1 = registry
            .worker_identity_for_claim("worker-1")
            .await
            .unwrap();
        let worker_2 = registry
            .worker_identity_for_claim("worker-2")
            .await
            .unwrap();

        assert_eq!(worker_1.worker_id, WorkerId::new("worker-1"));
        assert_eq!(worker_2.worker_id, WorkerId::new("worker-2"));
        assert_eq!(worker_1.host_id, WorkerHostId::new("test-host"));
        assert_eq!(worker_2.host_id, WorkerHostId::new("test-host"));
    }

    #[tokio::test]
    async fn missed_heartbeat_marks_worker_unavailable_for_claims() {
        let registry = heartbeat_test_registry();
        registry
            .tick_worker_heartbeat(test_registration("worker-1"))
            .await;

        time::sleep(Duration::from_millis(15)).await;
        registry.update_worker_liveness().await;

        let error = registry
            .worker_identity_for_claim("worker-1")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("missed heartbeat"));
    }

    #[tokio::test]
    async fn heartbeat_renewal_clears_missed_heartbeat() {
        let registry = heartbeat_test_registry();
        registry
            .tick_worker_heartbeat(test_registration("worker-1"))
            .await;

        time::sleep(Duration::from_millis(15)).await;
        registry.update_worker_liveness().await;
        registry
            .tick_worker_heartbeat(test_registration("worker-1"))
            .await;

        assert!(registry.worker_identity_for_claim("worker-1").await.is_ok());
    }

    #[tokio::test]
    async fn missed_heartbeat_threshold_deregisters_lost_host() {
        let registry = heartbeat_test_registry();
        registry
            .tick_worker_heartbeat(test_registration("worker-1"))
            .await;

        time::sleep(Duration::from_millis(35)).await;
        let lost_hosts = registry.update_worker_liveness().await;

        assert_eq!(lost_hosts, vec![WorkerHostId::new("test-host")]);
        assert!(
            registry
                .worker_identity_for_claim("worker-1")
                .await
                .unwrap_err()
                .to_string()
                .contains("not registered")
        );
    }

    #[tokio::test]
    async fn deregistering_one_worker_does_not_lose_host_with_remaining_registration() {
        let registry = heartbeat_test_registry();
        registry
            .tick_worker_heartbeat(test_registration_for_host("worker-1", "host-a"))
            .await;
        time::sleep(Duration::from_millis(20)).await;
        registry
            .tick_worker_heartbeat(test_registration_for_host("worker-2", "host-a"))
            .await;

        time::sleep(Duration::from_millis(15)).await;
        let lost_hosts = registry.update_worker_liveness().await;

        assert!(lost_hosts.is_empty());
        assert!(
            registry
                .worker_identity_for_claim("worker-1")
                .await
                .is_err()
        );
        assert!(registry.workers.read().await.contains_key("worker-2"));
    }

    #[tokio::test]
    async fn select_eligible_host_returns_registered_non_suspicious_host() {
        let registry = WorkerRegistry::new();
        registry
            .register_worker(test_registration_for_host("worker-z", "host-z"))
            .await;
        registry
            .register_worker(test_registration_for_host("worker-a", "host-a"))
            .await;

        assert_eq!(
            registry.select_eligible_host().await,
            Some(WorkerHostId::new("host-a"))
        );
    }

    #[tokio::test]
    async fn worker_count_tracks_registered_workers() {
        let registry = WorkerRegistry::new();
        assert_eq!(registry.worker_count().await, 0);

        registry
            .register_worker(test_registration_for_host("worker-1", "host-a"))
            .await;

        assert_eq!(registry.worker_count().await, 1);
    }

    #[tokio::test]
    async fn force_retry_host_reassigns_when_existing_host_is_not_eligible() {
        let registry = heartbeat_test_registry();
        registry
            .register_worker(test_registration_for_host("worker-1", "host-a"))
            .await;
        time::sleep(Duration::from_millis(15)).await;
        registry
            .register_worker(test_registration_for_host("worker-2", "host-b"))
            .await;

        assert_eq!(
            registry
                .select_force_retry_host(Some(&WorkerHostId::new("host-a")))
                .await,
            Some(WorkerHostId::new("host-b"))
        );
    }
}
