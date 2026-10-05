//! Realtime fan-out hub. Every browser holds one WebSocket; the hub tracks
//! which connection belongs to which user and which topics it follows, and
//! routes pre-serialised JSON messages to them.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use shared::ws::{ServerMsg, Topic};
use shared::UserSummary;
use tokio::sync::mpsc;
use uuid::Uuid;

pub type ConnId = u64;

/// Messages queued per connection before we start dropping for that client.
const CONN_BUFFER: usize = 256;

#[derive(Clone, Default)]
pub struct Hub {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    next_id: AtomicU64,
    state: RwLock<State>,
}

#[derive(Default)]
struct State {
    conns: HashMap<ConnId, Conn>,
    topics: HashMap<Topic, HashSet<ConnId>>,
    users: HashMap<Uuid, HashSet<ConnId>>,
}

struct Conn {
    user: Option<UserSummary>,
    tx: mpsc::Sender<Arc<str>>,
    topics: HashSet<Topic>,
}

/// Result of a disconnect, so the caller can emit presence changes.
pub struct Disconnected {
    pub user: Option<UserSummary>,
    /// True if this was the user's last open connection.
    pub went_offline: bool,
    pub topics: Vec<Topic>,
}

fn encode(msg: &ServerMsg) -> Option<Arc<str>> {
    match serde_json::to_string(msg) {
        Ok(s) => Some(Arc::from(s)),
        Err(e) => {
            tracing::error!(error = %e, "failed to encode realtime message");
            None
        }
    }
}

impl Hub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a connection. Returns its id, the receiving end of its
    /// outbound queue and whether the user just came online.
    pub fn connect(&self, user: Option<UserSummary>) -> (ConnId, mpsc::Receiver<Arc<str>>, bool) {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::channel(CONN_BUFFER);
        let mut st = self.inner.state.write().expect("hub poisoned");
        let mut came_online = false;
        if let Some(u) = &user {
            let set = st.users.entry(u.id).or_default();
            came_online = set.is_empty();
            set.insert(id);
        }
        st.conns.insert(
            id,
            Conn {
                user,
                tx,
                topics: HashSet::new(),
            },
        );
        (id, rx, came_online)
    }

    pub fn disconnect(&self, id: ConnId) -> Disconnected {
        let mut st = self.inner.state.write().expect("hub poisoned");
        let Some(conn) = st.conns.remove(&id) else {
            return Disconnected {
                user: None,
                went_offline: false,
                topics: Vec::new(),
            };
        };
        for topic in &conn.topics {
            if let Some(set) = st.topics.get_mut(topic) {
                set.remove(&id);
                if set.is_empty() {
                    st.topics.remove(topic);
                }
            }
        }
        let mut went_offline = false;
        if let Some(u) = &conn.user {
            if let Some(set) = st.users.get_mut(&u.id) {
                set.remove(&id);
                if set.is_empty() {
                    st.users.remove(&u.id);
                    went_offline = true;
                }
            }
        }
        Disconnected {
            user: conn.user,
            went_offline,
            topics: conn.topics.into_iter().collect(),
        }
    }

    /// Returns true if the connection was not yet subscribed.
    pub fn subscribe(&self, id: ConnId, topic: Topic) -> bool {
        let mut st = self.inner.state.write().expect("hub poisoned");
        let Some(conn) = st.conns.get_mut(&id) else {
            return false;
        };
        if !conn.topics.insert(topic.clone()) {
            return false;
        }
        st.topics.entry(topic).or_default().insert(id);
        true
    }

    pub fn unsubscribe(&self, id: ConnId, topic: &Topic) -> bool {
        let mut st = self.inner.state.write().expect("hub poisoned");
        let removed = st
            .conns
            .get_mut(&id)
            .map(|c| c.topics.remove(topic))
            .unwrap_or(false);
        if removed {
            if let Some(set) = st.topics.get_mut(topic) {
                set.remove(&id);
                if set.is_empty() {
                    st.topics.remove(topic);
                }
            }
        }
        removed
    }

    fn send_raw(st: &State, ids: impl IntoIterator<Item = ConnId>, payload: &Arc<str>) {
        for id in ids {
            if let Some(conn) = st.conns.get(&id) {
                // A full queue means the client is not keeping up; dropping
                // the message is better than blocking every other client.
                let _ = conn.tx.try_send(payload.clone());
            }
        }
    }

    pub fn publish(&self, topic: &Topic, msg: &ServerMsg) {
        let Some(payload) = encode(msg) else { return };
        let st = self.inner.state.read().expect("hub poisoned");
        if let Some(ids) = st.topics.get(topic) {
            Self::send_raw(&st, ids.iter().copied(), &payload);
        }
    }

    /// Publishes to a topic, skipping the given connection (e.g. the sender).
    pub fn publish_except(&self, topic: &Topic, msg: &ServerMsg, except: ConnId) {
        let Some(payload) = encode(msg) else { return };
        let st = self.inner.state.read().expect("hub poisoned");
        if let Some(ids) = st.topics.get(topic) {
            Self::send_raw(&st, ids.iter().copied().filter(|id| *id != except), &payload);
        }
    }

    pub fn send_user(&self, user_id: Uuid, msg: &ServerMsg) {
        let Some(payload) = encode(msg) else { return };
        let st = self.inner.state.read().expect("hub poisoned");
        if let Some(ids) = st.users.get(&user_id) {
            Self::send_raw(&st, ids.iter().copied(), &payload);
        }
    }

    pub fn send_conn(&self, id: ConnId, msg: &ServerMsg) {
        let Some(payload) = encode(msg) else { return };
        let st = self.inner.state.read().expect("hub poisoned");
        Self::send_raw(&st, [id], &payload);
    }

    pub fn broadcast(&self, msg: &ServerMsg) {
        let Some(payload) = encode(msg) else { return };
        let st = self.inner.state.read().expect("hub poisoned");
        Self::send_raw(&st, st.conns.keys().copied(), &payload);
    }

    pub fn online_users(&self) -> Vec<Uuid> {
        let st = self.inner.state.read().expect("hub poisoned");
        st.users.keys().copied().collect()
    }

    pub fn online_count(&self) -> usize {
        self.inner.state.read().expect("hub poisoned").users.len()
    }

    pub fn is_online(&self, user_id: Uuid) -> bool {
        self.inner
            .state
            .read()
            .expect("hub poisoned")
            .users
            .contains_key(&user_id)
    }

    /// Distinct logged-in users currently subscribed to a topic.
    pub fn topic_users(&self, topic: &Topic) -> Vec<UserSummary> {
        let st = self.inner.state.read().expect("hub poisoned");
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        if let Some(ids) = st.topics.get(topic) {
            for id in ids {
                if let Some(u) = st.conns.get(id).and_then(|c| c.user.as_ref()) {
                    if seen.insert(u.id) {
                        out.push(u.clone());
                    }
                }
            }
        }
        out.sort_by_key(|a| a.display_name.to_lowercase());
        out
    }

    /// Number of connections (including guests) on a topic.
    pub fn topic_conn_count(&self, topic: &Topic) -> usize {
        let st = self.inner.state.read().expect("hub poisoned");
        st.topics.get(topic).map(|s| s.len()).unwrap_or(0)
    }

    /// Whether a specific user has any connection subscribed to a topic.
    pub fn user_in_topic(&self, user_id: Uuid, topic: &Topic) -> bool {
        let st = self.inner.state.read().expect("hub poisoned");
        st.users.get(&user_id).is_some_and(|ids| {
            ids.iter()
                .any(|id| st.conns.get(id).is_some_and(|c| c.topics.contains(topic)))
        })
    }

    /// Connections of a user that follow a topic (e.g. to kick them).
    pub fn user_conns_in_topic(&self, user_id: Uuid, topic: &Topic) -> Vec<ConnId> {
        let st = self.inner.state.read().expect("hub poisoned");
        st.users
            .get(&user_id)
            .map(|ids| {
                ids.iter()
                    .copied()
                    .filter(|id| st.conns.get(id).is_some_and(|c| c.topics.contains(topic)))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn conn_user(&self, id: ConnId) -> Option<UserSummary> {
        let st = self.inner.state.read().expect("hub poisoned");
        st.conns.get(&id).and_then(|c| c.user.clone())
    }

    /// All topics of a given kind that currently have subscribers.
    pub fn active_topics(&self, pred: impl Fn(&Topic) -> bool) -> Vec<Topic> {
        let st = self.inner.state.read().expect("hub poisoned");
        st.topics.keys().filter(|t| pred(t)).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::Role;

    fn user(name: &str) -> UserSummary {
        UserSummary {
            id: Uuid::now_v7(),
            username: name.into(),
            display_name: name.into(),
            avatar_url: None,
            role: Role::Member,
        }
    }

    #[tokio::test]
    async fn routes_messages_to_topic_and_user() {
        let hub = Hub::new();
        let alice = user("alice");
        let (a1, mut rx_a1, online) = hub.connect(Some(alice.clone()));
        assert!(online);
        let (_a2, mut rx_a2, online2) = hub.connect(Some(alice.clone()));
        assert!(!online2);
        let (g, mut rx_g, _) = hub.connect(None);

        let topic = Topic::ForumTopic(Uuid::now_v7());
        assert!(hub.subscribe(a1, topic.clone()));
        assert!(!hub.subscribe(a1, topic.clone()));
        hub.subscribe(g, topic.clone());

        hub.publish(&topic, &ServerMsg::Error { message: "x".into() });
        assert!(rx_a1.try_recv().is_ok());
        assert!(rx_g.try_recv().is_ok());
        assert!(rx_a2.try_recv().is_err());

        hub.send_user(alice.id, &ServerMsg::Error { message: "y".into() });
        assert!(rx_a1.try_recv().is_ok());
        assert!(rx_a2.try_recv().is_ok());
        assert_eq!(hub.topic_users(&topic).len(), 1);
        assert_eq!(hub.topic_conn_count(&topic), 2);

        let d = hub.disconnect(a1);
        assert!(!d.went_offline);
        assert_eq!(hub.topic_conn_count(&topic), 1);
    }
}
