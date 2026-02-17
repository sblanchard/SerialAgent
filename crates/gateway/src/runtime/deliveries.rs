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
        // Return most recent first
        let items: Vec<Delivery> = inner
            .iter()
            .rev()
            .collect::<Vec<_>>()
            .into_iter()
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

    pub async fn unread_count(&self) -> usize {
        self.inner.read().await.iter().filter(|d| !d.read).count()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DeliveryEvent> {
        self.event_tx.subscribe()
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
