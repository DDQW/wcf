//! Browser side of the multiplexed realtime connection (`/ws`).
//!
//! One WebSocket per tab. Components subscribe to topics and register
//! message handlers for the lifetime of their reactive owner; the client
//! reconnects with backoff and re-subscribes automatically. On the server
//! (SSR) every method is a no-op.

use std::collections::HashSet;

use leptos::prelude::*;
use shared::ws::{ClientMsg, ServerMsg, Topic};
use uuid::Uuid;

use crate::state::Session;

#[cfg(feature = "hydrate")]
mod client {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use shared::ws::{ServerMsg, Topic};
    use wasm_bindgen::prelude::*;

    pub type Handler = Rc<dyn Fn(&ServerMsg)>;

    #[derive(Default)]
    pub struct Inner {
        pub ws: Option<web_sys::WebSocket>,
        pub generation: u64,
        pub handlers: Vec<(u64, Handler)>,
        pub next_handler: u64,
        pub subs: HashMap<Topic, usize>,
        pub retry: u32,
        pub started: bool,
        pub closures: Vec<Closure<dyn FnMut(JsValue)>>,
        /// Smallest round trip seen recently; its sample wins (NTP-style).
        pub best_rtt: f64,
    }

    pub type Shared = Rc<RefCell<Inner>>;
}

#[derive(Clone, Copy)]
pub struct Realtime {
    pub connected: RwSignal<bool>,
    pub online: RwSignal<HashSet<Uuid>>,
    /// Estimated server clock minus local clock (ms).
    pub clock_offset: RwSignal<f64>,
    /// Incremented on every successful (re)connect so pages can resync.
    pub epoch: RwSignal<u64>,
    #[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
    session: Session,
    #[cfg(feature = "hydrate")]
    inner: StoredValue<client::Shared, LocalStorage>,
}

impl Realtime {
    pub fn new(session: Session) -> Self {
        Self {
            connected: RwSignal::new(false),
            online: RwSignal::new(HashSet::new()),
            clock_offset: RwSignal::new(0.0),
            epoch: RwSignal::new(0),
            session,
            #[cfg(feature = "hydrate")]
            inner: StoredValue::new_local(Default::default()),
        }
    }

    /// Server time in milliseconds (local clock corrected by the offset).
    pub fn server_now(&self) -> f64 {
        crate::state::now_ms() + self.clock_offset.get_untracked()
    }

    pub fn is_online(&self, user_id: Uuid) -> bool {
        self.online.with(|s| s.contains(&user_id))
    }

    /// Opens the connection (client only, idempotent).
    pub fn start(&self) {
        #[cfg(feature = "hydrate")]
        {
            let inner = self.inner.get_value();
            if inner.borrow().started {
                return;
            }
            inner.borrow_mut().started = true;
            self.connect();
            let rt = *self;
            // Keepalive + clock sync.
            let _ = set_interval_with_handle(
                move || rt.send(&ClientMsg::Ping { client_ts: crate::state::now_ms() }),
                std::time::Duration::from_secs(20),
            );
        }
    }

    /// Drops the socket and connects again (e.g. after login/logout so the
    /// server sees the new session cookie).
    pub fn reconnect(&self) {
        #[cfg(feature = "hydrate")]
        {
            let inner = self.inner.get_value();
            if !inner.borrow().started {
                return;
            }
            let old = {
                let mut i = inner.borrow_mut();
                i.generation += 1;
                i.retry = 0;
                i.ws.take()
            };
            if let Some(ws) = old {
                // Detach handlers first: their closures are dropped when the
                // new socket's closures replace them, but the old socket
                // still fires `close` asynchronously.
                ws.set_onopen(None);
                ws.set_onmessage(None);
                ws.set_onclose(None);
                let _ = ws.close();
            }
            self.connected.set(false);
            self.connect();
        }
    }

    #[cfg(feature = "hydrate")]
    fn connect(&self) {
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        let inner = self.inner.get_value();
        let location = window().location();
        let proto = if location.protocol().ok().as_deref() == Some("https:") { "wss" } else { "ws" };
        let host = location.host().unwrap_or_default();
        let Ok(ws) = web_sys::WebSocket::new(&format!("{proto}://{host}/ws")) else {
            self.schedule_reconnect();
            return;
        };
        let generation = {
            let mut i = inner.borrow_mut();
            i.generation += 1;
            i.generation
        };
        let rt = *self;

        let on_open = Closure::<dyn FnMut(JsValue)>::new({
            let inner = inner.clone();
            move |_| {
                if inner.borrow().generation != generation {
                    return;
                }
                inner.borrow_mut().retry = 0;
                rt.connected.set(true);
                let topics: Vec<Topic> = inner.borrow().subs.keys().cloned().collect();
                for topic in topics {
                    rt.send(&ClientMsg::Subscribe { topic });
                }
                rt.send(&ClientMsg::Ping { client_ts: crate::state::now_ms() });
            }
        });
        let on_message = Closure::<dyn FnMut(JsValue)>::new({
            let inner = inner.clone();
            move |ev: JsValue| {
                if inner.borrow().generation != generation {
                    return;
                }
                let ev: web_sys::MessageEvent = ev.unchecked_into();
                if let Some(text) = ev.data().as_string() {
                    match serde_json::from_str::<ServerMsg>(&text) {
                        Ok(msg) => rt.handle(msg),
                        Err(e) => leptos::logging::warn!("realtime: unbekannte Nachricht: {e}"),
                    }
                }
            }
        });
        let on_close = Closure::<dyn FnMut(JsValue)>::new({
            let inner = inner.clone();
            move |_| {
                if inner.borrow().generation != generation {
                    return;
                }
                inner.borrow_mut().ws = None;
                rt.connected.set(false);
                rt.schedule_reconnect();
            }
        });
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        let mut i = inner.borrow_mut();
        i.ws = Some(ws);
        i.closures = vec![on_open, on_message, on_close];
    }

    #[cfg(feature = "hydrate")]
    fn schedule_reconnect(&self) {
        let inner = self.inner.get_value();
        let retry = {
            let mut i = inner.borrow_mut();
            i.retry = (i.retry + 1).min(6);
            i.retry
        };
        // 1s, 2s, 4s … capped at 30s, plus jitter.
        let base = (1u64 << (retry - 1)).min(30) * 1000;
        let jitter = (js_sys::Math::random() * 500.0) as u64;
        let rt = *self;
        set_timeout(
            move || {
                let inner = rt.inner.get_value();
                if inner.borrow().ws.is_none() {
                    rt.connect();
                }
            },
            std::time::Duration::from_millis(base + jitter),
        );
    }

    #[cfg(feature = "hydrate")]
    fn handle(&self, msg: ServerMsg) {
        match &msg {
            ServerMsg::Hello {
                online,
                server_ts,
                unread_notifications,
                ..
            } => {
                self.online.set(online.iter().copied().collect());
                self.clock_offset.set(server_ts - crate::state::now_ms());
                self.inner.get_value().borrow_mut().best_rtt = f64::INFINITY;
                self.session.unread.set(*unread_notifications);
                self.epoch.update(|e| *e += 1);
                self.sync_clock();
            }
            ServerMsg::Pong { client_ts, server_ts } => {
                let now = crate::state::now_ms();
                let rtt = (now - client_ts).max(0.0);
                let sample = server_ts + rtt / 2.0 - now;
                let inner = self.inner.get_value();
                let best = inner.borrow().best_rtt;
                if rtt <= best * 1.25 {
                    // A fast round trip is the most trustworthy sample.
                    self.clock_offset.set(sample);
                    inner.borrow_mut().best_rtt = rtt.max(1.0);
                } else {
                    // Slow samples only nudge; the best RTT ages so we adapt.
                    self.clock_offset.update(|o| *o = *o * 0.9 + sample * 0.1);
                    inner.borrow_mut().best_rtt = best * 1.1;
                }
            }
            ServerMsg::Presence { user_id, online } => {
                self.online.update(|s| {
                    if *online {
                        s.insert(*user_id);
                    } else {
                        s.remove(user_id);
                    }
                });
            }
            ServerMsg::Notification { unread, .. } | ServerMsg::NotificationsRead { unread } => {
                self.session.unread.set(*unread);
            }
            _ => {}
        }
        let handlers: Vec<client::Handler> = self
            .inner
            .get_value()
            .borrow()
            .handlers
            .iter()
            .map(|(_, h)| h.clone())
            .collect();
        for h in handlers {
            h(&msg);
        }
    }

    /// Sends a short burst of clock probes (e.g. when a watch room opens).
    pub fn sync_clock(&self) {
        #[cfg(feature = "hydrate")]
        {
            let rt = *self;
            leptos::task::spawn_local(async move {
                for _ in 0..5 {
                    rt.send(&ClientMsg::Ping { client_ts: crate::state::now_ms() });
                    crate::components::common::sleep_ms(200).await;
                }
            });
        }
    }

    pub fn send(&self, msg: &ClientMsg) {
        #[cfg(feature = "hydrate")]
        {
            let inner = self.inner.get_value();
            let i = inner.borrow();
            if let Some(ws) = &i.ws {
                if ws.ready_state() == web_sys::WebSocket::OPEN {
                    if let Ok(text) = serde_json::to_string(msg) {
                        let _ = ws.send_with_str(&text);
                    }
                }
            }
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = msg;
    }

    pub fn subscribe(&self, topic: Topic) {
        #[cfg(feature = "hydrate")]
        {
            let inner = self.inner.get_value();
            let first = {
                let mut i = inner.borrow_mut();
                let count = i.subs.entry(topic.clone()).or_insert(0);
                *count += 1;
                *count == 1
            };
            if first {
                self.send(&ClientMsg::Subscribe { topic });
            }
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = topic;
    }

    pub fn unsubscribe(&self, topic: Topic) {
        #[cfg(feature = "hydrate")]
        {
            let inner = self.inner.get_value();
            let last = {
                let mut i = inner.borrow_mut();
                match i.subs.get_mut(&topic) {
                    Some(count) if *count > 1 => {
                        *count -= 1;
                        false
                    }
                    Some(_) => {
                        i.subs.remove(&topic);
                        true
                    }
                    None => false,
                }
            };
            if last {
                self.send(&ClientMsg::Unsubscribe { topic });
            }
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = topic;
    }

    /// Subscribes for the lifetime of the current reactive owner.
    pub fn subscribe_scoped(&self, topic: Topic) {
        self.subscribe(topic.clone());
        let rt = *self;
        on_cleanup(move || rt.unsubscribe(topic));
    }

    /// Registers a handler for all incoming messages until the current
    /// reactive owner is cleaned up.
    pub fn on_message(&self, handler: impl Fn(&ServerMsg) + 'static) {
        #[cfg(feature = "hydrate")]
        {
            let inner = self.inner.get_value();
            let id = {
                let mut i = inner.borrow_mut();
                i.next_handler += 1;
                let id = i.next_handler;
                i.handlers.push((id, std::rc::Rc::new(handler)));
                id
            };
            let rt = *self;
            on_cleanup(move || {
                rt.inner
                    .get_value()
                    .borrow_mut()
                    .handlers
                    .retain(|(h, _)| *h != id);
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = handler;
    }
}

pub fn use_realtime() -> Realtime {
    expect_context::<Realtime>()
}
