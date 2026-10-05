//! The `/ws` endpoint: authenticates via the session cookie, registers the
//! connection with the hub and dispatches client messages.

use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use shared::ws::{ClientMsg, ServerMsg, Topic};
use shared::AuthUser;
use tower_sessions::Session;

use crate::hub::ConnId;
use crate::state::AppState;
use crate::{auth, forum, notifications, users};

pub fn now_ms() -> f64 {
    chrono::Utc::now().timestamp_micros() as f64 / 1000.0
}

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>, session: Session) -> Response {
    let user = auth::user_from_session(&state.db, &session).await.ok().flatten();
    ws.max_message_size(64 * 1024)
        .on_upgrade(move |socket| run(socket, state, user))
}

async fn run(socket: WebSocket, state: AppState, user: Option<AuthUser>) {
    let (conn_id, mut rx, came_online) = state.hub.connect(user.as_ref().map(|u| u.summary()));
    let (mut sink, mut stream) = socket.split();

    if let Some(u) = &user {
        if came_online {
            state
                .hub
                .publish(&Topic::Presence, &ServerMsg::Presence { user_id: u.id, online: true });
        }
        let _ = users::touch_last_seen(&state.db, u.id).await;
    }
    let unread = match &user {
        Some(u) => notifications::unread_count(&state.db, u.id).await.unwrap_or(0),
        None => 0,
    };
    state.hub.send_conn(
        conn_id,
        &ServerMsg::Hello {
            user_id: user.as_ref().map(|u| u.id),
            server_ts: now_ms(),
            online: state.hub.online_users(),
            unread_notifications: unread,
        },
    );

    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(25));
        ping.tick().await;
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(text) => {
                        if sink.send(Message::Text(text.as_ref().into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Bytes::new())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(text) => match serde_json::from_str::<ClientMsg>(text.as_str()) {
                Ok(m) => dispatch(&state, conn_id, user.as_ref(), m).await,
                Err(_) => state.hub.send_conn(
                    conn_id,
                    &ServerMsg::Error {
                        message: "Unbekannte Nachricht".into(),
                    },
                ),
            },
            Message::Close(_) => break,
            _ => {}
        }
    }

    writer.abort();
    let gone = state.hub.disconnect(conn_id);
    for topic in &gone.topics {
        crate::realtime_ext::on_leave(&state, conn_id, user.as_ref(), topic).await;
    }
    if let Some(u) = &gone.user {
        if gone.went_offline {
            state
                .hub
                .publish(&Topic::Presence, &ServerMsg::Presence { user_id: u.id, online: false });
            crate::realtime_ext::on_offline(&state, u.id).await;
        }
        let _ = users::touch_last_seen(&state.db, u.id).await;
    }
}

async fn can_subscribe(state: &AppState, user: Option<&AuthUser>, topic: &Topic) -> bool {
    match topic {
        Topic::Presence => true,
        Topic::ForumTopic(id) => forum::can_read_topic(&state.db, user, *id)
            .await
            .unwrap_or(false),
        other => crate::realtime_ext::can_subscribe(state, user, other).await,
    }
}

async fn dispatch(state: &AppState, conn_id: ConnId, user: Option<&AuthUser>, msg: ClientMsg) {
    match msg {
        ClientMsg::Subscribe { topic } => {
            if can_subscribe(state, user, &topic).await {
                if state.hub.subscribe(conn_id, topic.clone()) {
                    crate::realtime_ext::on_join(state, conn_id, user, &topic).await;
                }
            } else {
                state.hub.send_conn(
                    conn_id,
                    &ServerMsg::Error {
                        message: "Kein Zugriff auf diesen Kanal.".into(),
                    },
                );
            }
        }
        ClientMsg::Unsubscribe { topic } => {
            if state.hub.unsubscribe(conn_id, &topic) {
                crate::realtime_ext::on_leave(state, conn_id, user, &topic).await;
            }
        }
        ClientMsg::Ping { client_ts } => state.hub.send_conn(
            conn_id,
            &ServerMsg::Pong {
                client_ts,
                server_ts: now_ms(),
            },
        ),
        #[allow(unreachable_patterns)]
        other => crate::realtime_ext::dispatch(state, conn_id, user, other).await,
    }
}
