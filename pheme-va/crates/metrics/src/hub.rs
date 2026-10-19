use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crate::MetricEvent;

/// Consumer of events emitted by a [`MetricsHub`]. Subscribers should do small,
/// non-blocking work here; a host can enqueue the event for an asynchronous
/// exporter if sending it elsewhere may block.
pub trait MetricsSubscriber: Send + Sync {
    fn on_event(&self, event: &MetricEvent);
}

impl<T> MetricsSubscriber for Arc<T>
where
    T: MetricsSubscriber + ?Sized,
{
    fn on_event(&self, event: &MetricEvent) {
        (**self).on_event(event);
    }
}

struct SubscriberEntry {
    id: u64,
    subscriber: Arc<dyn MetricsSubscriber>,
}

/// Synchronous, host-independent publish/subscribe hub for metric events.
///
/// The hub does not know about HTTP, Go, a TUI, or any particular runtime. A
/// caller may attach multiple subscribers for local displays, tests, batching,
/// or a host-level transport adapter.
pub struct MetricsHub {
    next_subscriber_id: AtomicU64,
    subscribers: Mutex<Vec<SubscriberEntry>>,
}

impl MetricsHub {
    pub fn new() -> Self {
        Self {
            next_subscriber_id: AtomicU64::new(0),
            subscribers: Mutex::new(Vec::new()),
        }
    }

    /// Register a subscriber. The returned handle removes it when dropped.
    pub fn subscribe<S>(self: &Arc<Self>, subscriber: S) -> MetricsSubscription
    where
        S: MetricsSubscriber + 'static,
    {
        let id = self.next_subscriber_id.fetch_add(1, Ordering::Relaxed);
        let entry = SubscriberEntry {
            id,
            subscriber: Arc::new(subscriber),
        };
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(entry);
        }
        MetricsSubscription {
            hub: Arc::downgrade(self),
            id,
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .map(|items| items.len())
            .unwrap_or(0)
    }

    pub(crate) fn unsubscribe(&self, id: u64) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|entry| entry.id != id);
        }
    }

    pub fn publish(&self, event: &MetricEvent) {
        let subscribers = self
            .subscribers
            .lock()
            .map(|items| {
                items
                    .iter()
                    .map(|entry| Arc::clone(&entry.subscriber))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        for subscriber in subscribers {
            subscriber.on_event(event);
        }
    }
}

impl Default for MetricsHub {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII subscription handle returned from [`MetricsHub::subscribe`].
pub struct MetricsSubscription {
    hub: Weak<MetricsHub>,
    id: u64,
}

impl MetricsSubscription {
    pub fn unsubscribe(self) {
        // Drop performs the same operation. Consuming the handle makes this
        // method convenient when a host wants to stop a subscriber explicitly.
    }
}

impl Drop for MetricsSubscription {
    fn drop(&mut self) {
        if let Some(hub) = self.hub.upgrade() {
            hub.unsubscribe(self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{MetricScope, MetricUnit, MetricValue, METRICS_SCHEMA_VERSION};

    #[derive(Default)]
    struct RecordingSubscriber {
        events: Mutex<Vec<String>>,
    }

    impl MetricsSubscriber for RecordingSubscriber {
        fn on_event(&self, event: &MetricEvent) {
            self.events.lock().unwrap().push(event.name.clone());
        }
    }

    fn event(name: &str) -> MetricEvent {
        MetricEvent {
            schema_version: METRICS_SCHEMA_VERSION,
            experiment_id: None,
            run_id: "run-1".to_owned(),
            sequence: 0,
            timestamp_ms: 1,
            name: name.to_owned(),
            value: Some(MetricValue::Integer(1)),
            unit: MetricUnit::Count,
            scope: MetricScope::Run,
            source: "test".to_owned(),
            unavailable_reason: None,
        }
    }

    #[test]
    fn broadcasts_to_subscribers_and_unsubscribes_on_drop() {
        let hub = Arc::new(MetricsHub::new());
        let subscriber = Arc::new(RecordingSubscriber::default());
        let handle = hub.subscribe(Arc::clone(&subscriber));
        assert_eq!(hub.subscriber_count(), 1);

        hub.publish(&event("first"));
        assert_eq!(subscriber.events.lock().unwrap().as_slice(), &["first"]);

        drop(handle);
        assert_eq!(hub.subscriber_count(), 0);
        hub.publish(&event("second"));
        assert_eq!(subscriber.events.lock().unwrap().as_slice(), &["first"]);
    }
}
