//! Delivery store — in-app notification/delivery system for scheduled job results.
//!
//! Deliveries are the output of scheduled runs: digest summaries, alerts, etc.
//! They are persisted to JSONL and kept in a bounded in-memory ring.

use std::collections::VecDeque;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use super::schedules::DeliveryTarget;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Delivery model
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Delivery {
    pub id: Uuid,
    pub schedule_id: Option<Uuid>,
    pub schedule_name: Option<String>,
    pub run_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub title: String,
    pub body: String,
    /// Source URLs or identifiers used to produce this delivery
    pub sources: Vec<String>,
    pub read: bool,
    /// Token usage for this delivery's run.
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
    pub metadata: serde_json::Value,
}

impl Delivery {
    pub fn new(title: String, body: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            schedule_id: None,
            schedule_name: None,
            run_id: None,
            created_at: Utc::now(),
            title,
            body,
            sources: Vec::new(),
            read: false,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            metadata: serde_json::Value::Null,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Delivery events (for SSE)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeliveryEvent {
    NewDelivery { delivery: Delivery },
    DeliveryRead { id: Uuid },
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DeliveryStore
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

const MAX_DELIVERIES: usize = 1000;

pub struct DeliveryStore {
    inner: RwLock<VecDeque<Delivery>>,
    persist_path: PathBuf,
    event_tx: broadcast::Sender<DeliveryEvent>,
}

impl DeliveryStore {
    pub fn new(state_path: &std::path::Path) -> Self {
        let persist_path = state_path.join("deliveries.jsonl");
        let (event_tx, _) = broadcast::channel(64);

        let mut store = Self {
            inner: RwLock::new(VecDeque::new()),
            persist_path,
            event_tx,
        };
        store.load();
        store
    }

    fn load(&mut self) {
        if let Ok(data) = std::fs::read_to_string(&self.persist_path) {
            let mut deliveries = VecDeque::new();
            for line in data.lines() {
                if let Ok(d) = serde_json::from_str::<Delivery>(line) {
                    deliveries.push_back(d);
                }
            }
            let original_count = deliveries.len();
            // Keep only the most recent
            while deliveries.len() > MAX_DELIVERIES {
                deliveries.pop_front();
            }
            let count = deliveries.len();
            // Truncate JSONL on disk if we trimmed entries.
            if count < original_count {
                Self::rewrite_jsonl(&self.persist_path, &deliveries);
            }
            self.inner = RwLock::new(deliveries);
            if count > 0 {
                tracing::info!(count, "loaded deliveries from disk");
            }
        }
    }

    /// Rewrite the entire JSONL file from the in-memory ring.
    fn rewrite_jsonl(path: &std::path::Path, deliveries: &VecDeque<Delivery>) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("jsonl.tmp");
        let mut ok = false;
        if let Ok(mut f) = std::fs::File::create(&tmp) {
            use std::io::Write;
            ok = true;
            for d in deliveries {
                if let Ok(json) = serde_json::to_string(d) {
                    if writeln!(f, "{}", json).is_err() {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if ok {
            let _ = std::fs::rename(&tmp, path);
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    fn persist_one(path: &std::path::Path, delivery: &Delivery) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(delivery) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "{}", json);
            }
        }
    }

    pub async fn insert(&self, delivery: Delivery) -> Delivery {
        let d = delivery.clone();
        let mut inner = self.inner.write().await;
        inner.push_back(delivery.clone());
        // Bound the ring
        while inner.len() > MAX_DELIVERIES {
            inner.pop_front();
        }
        drop(inner);

        Self::persist_one(&self.persist_path, &d);
        let _ = self.event_tx.send(DeliveryEvent::NewDelivery {
            delivery: d.clone(),
        });
        d
    }

    pub async fn list(&self, limit: usize, offset: usize) -> (Vec<Delivery>, usize) {
        let inner = self.inner.read().await;
        let total = inner.len();
        // Return most recent first — skip/take directly on the reversed
        // iterator to avoid an intermediate Vec allocation.
        let items: Vec<Delivery> = inner
            .iter()
            .rev()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();
        (items, total)
    }

    pub async fn get(&self, id: &Uuid) -> Option<Delivery> {
        self.inner
            .read()
            .await
            .iter()
            .find(|d| d.id == *id)
            .cloned()
    }

    pub async fn mark_read(&self, id: &Uuid) -> bool {
        let mut inner = self.inner.write().await;
        if let Some(d) = inner.iter_mut().find(|d| d.id == *id) {
            d.read = true;
            // Persist the read state to disk so it survives restarts.
            Self::rewrite_jsonl(&self.persist_path, &inner);
            let _ = self.event_tx.send(DeliveryEvent::DeliveryRead { id: *id });
            true
        } else {
            false
        }
    }

    /// List deliveries scoped to a specific schedule.
    pub async fn list_by_schedule(
        &self,
        schedule_id: &Uuid,
        limit: usize,
        offset: usize,
    ) -> (Vec<Delivery>, usize) {
        let inner = self.inner.read().await;
        let matching: Vec<&Delivery> = inner
            .iter()
            .filter(|d| d.schedule_id.as_ref() == Some(schedule_id))
            .collect();
        let total = matching.len();
        let items: Vec<Delivery> = matching
            .into_iter()
            .rev()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();
        (items, total)
    }

    /// List deliveries and compute unread count under a single lock acquisition.
    pub async fn list_with_unread(
        &self,
        limit: usize,
        offset: usize,
    ) -> (Vec<Delivery>, usize, usize) {
        let inner = self.inner.read().await;
        let total = inner.len();
        let unread = inner.iter().filter(|d| !d.read).count();
        let items: Vec<Delivery> = inner
            .iter()
            .rev()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();
        (items, total, unread)
    }

    pub async fn unread_count(&self) -> usize {
        self.inner.read().await.iter().filter(|d| !d.read).count()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DeliveryEvent> {
        self.event_tx.subscribe()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Webhook dispatcher
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Fire-and-forget: POST delivery content to all webhook targets.
/// Spawns one task per webhook URL. Logs errors but never fails the caller.
///
/// `user_agent` overrides the default User-Agent header if provided.
pub fn dispatch_webhooks(delivery: &Delivery, targets: &[DeliveryTarget], user_agent: Option<&str>) {
    let webhook_urls: Vec<String> = targets
        .iter()
        .filter_map(|t| match t {
            DeliveryTarget::Webhook { url } => Some(url.clone()),
            _ => None,
        })
        .collect();

    if webhook_urls.is_empty() {
        return;
    }

    let payload = serde_json::json!({
        "delivery_id": delivery.id,
        "schedule_id": delivery.schedule_id,
        "schedule_name": delivery.schedule_name,
        "run_id": delivery.run_id,
        "title": delivery.title,
        "body": delivery.body,
        "sources": delivery.sources,
        "created_at": delivery.created_at,
    });

    let ua = user_agent.unwrap_or("SerialAgent-Webhook/1.0").to_string();
    // Derive jitter seed from delivery ID to avoid thundering herd on retries.
    let jitter_seed = delivery.id.as_bytes()[15] as u64;

    for url in webhook_urls {
        let payload = payload.clone();
        let ua = ua.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default();

            const MAX_ATTEMPTS: u32 = 3;
            for attempt in 1..=MAX_ATTEMPTS {
                match client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .header("User-Agent", &ua)
                    .json(&payload)
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        tracing::info!(url = %url, status = %resp.status(), attempt, "webhook delivered");
                        return;
                    }
                    Ok(resp) if resp.status().is_server_error() && attempt < MAX_ATTEMPTS => {
                        tracing::warn!(
                            url = %url,
                            status = %resp.status(),
                            attempt,
                            "webhook 5xx, will retry"
                        );
                    }
                    Ok(resp) => {
                        tracing::warn!(
                            url = %url,
                            status = %resp.status(),
                            attempt,
                            "webhook returned non-success status"
                        );
                        return; // 4xx or final 5xx — don't retry
                    }
                    Err(e) if attempt < MAX_ATTEMPTS => {
                        tracing::warn!(url = %url, error = %e, attempt, "webhook failed, will retry");
                    }
                    Err(e) => {
                        tracing::warn!(url = %url, error = %e, attempt, "webhook delivery failed after retries");
                        return;
                    }
                }
                // Exponential back-off with jitter: base 1s/2s + 0-255ms jitter
                let base_ms = (1u64 << (attempt - 1)) * 1000;
                let jitter_ms = (jitter_seed.wrapping_mul(attempt as u64 * 37)) % 256;
                tokio::time::sleep(std::time::Duration::from_millis(base_ms + jitter_ms)).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delivery_insert_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeliveryStore::new(dir.path());

        let d = Delivery::new("Test Title".into(), "Test body".into());
        store.insert(d).await;

        let (items, total) = store.list(10, 0).await;
        assert_eq!(total, 1);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Test Title");
    }

    #[tokio::test]
    async fn delivery_mark_read() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeliveryStore::new(dir.path());

        let d = Delivery::new("Test".into(), "Body".into());
        let id = d.id;
        store.insert(d).await;

        assert_eq!(store.unread_count().await, 1);
        store.mark_read(&id).await;
        assert_eq!(store.unread_count().await, 0);
    }

    #[tokio::test]
    async fn delivery_mark_read_persists() {
        let dir = tempfile::tempdir().unwrap();
        let id = {
            let store = DeliveryStore::new(dir.path());
            let d = Delivery::new("Persist Read".into(), "Body".into());
            let id = d.id;
            store.insert(d).await;
            store.mark_read(&id).await;
            id
        };
        // Reload from disk — read flag should be preserved.
        let store2 = DeliveryStore::new(dir.path());
        let d = store2.get(&id).await.unwrap();
        assert!(d.read, "read flag should survive reload");
    }

    #[tokio::test]
    async fn delivery_list_by_schedule() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeliveryStore::new(dir.path());
        let sched_id = Uuid::new_v4();

        let mut d1 = Delivery::new("Match".into(), "body".into());
        d1.schedule_id = Some(sched_id);
        store.insert(d1).await;

        let d2 = Delivery::new("NoMatch".into(), "body".into());
        store.insert(d2).await;

        let (items, total) = store.list_by_schedule(&sched_id, 10, 0).await;
        assert_eq!(total, 1);
        assert_eq!(items[0].title, "Match");
    }

    #[tokio::test]
    async fn delivery_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeliveryStore::new(dir.path());

        for i in 0..1010 {
            let d = Delivery::new(format!("D{}", i), "body".into());
            store.insert(d).await;
        }

        let (_, total) = store.list(10, 0).await;
        assert!(total <= MAX_DELIVERIES);
    }
}
