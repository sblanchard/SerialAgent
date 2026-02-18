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
            // Keep only the most recent
            while deliveries.len() > MAX_DELIVERIES {
                deliveries.pop_front();
            }
            let count = deliveries.len();
            self.inner = RwLock::new(deliveries);
            if count > 0 {
                tracing::info!(count, "loaded deliveries from disk");
            }
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
            let _ = self.event_tx.send(DeliveryEvent::DeliveryRead { id: *id });
            true
        } else {
            false
        }
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

/// Webhook POST payload.
#[derive(Serialize)]
struct WebhookPayload<'a> {
    delivery_id: Uuid,
    schedule_id: Option<Uuid>,
    schedule_name: Option<&'a str>,
    run_id: Option<Uuid>,
    title: &'a str,
    body: &'a str,
    sources: &'a [String],
    created_at: DateTime<Utc>,
}

/// Fire-and-forget: POST delivery content to all webhook targets.
/// Spawns one task per webhook URL. Logs errors but never fails the caller.
pub fn dispatch_webhooks(delivery: &Delivery, targets: &[DeliveryTarget]) {
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

    for url in webhook_urls {
        let payload = payload.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default();

            match client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("User-Agent", "SerialAgent-Webhook/1.0")
                .json(&payload)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    tracing::info!(url = %url, status = %resp.status(), "webhook delivered");
                }
                Ok(resp) => {
                    tracing::warn!(
                        url = %url,
                        status = %resp.status(),
                        "webhook returned non-success status"
                    );
                }
                Err(e) => {
                    tracing::warn!(url = %url, error = %e, "webhook delivery failed");
                }
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
