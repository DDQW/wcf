//! Realtime hooks of modules added in later phases (chat typing, watch
//! rooms, call signalling). The core `/ws` loop calls into these.

use std::time::Duration;

use shared::ws::{ClientMsg, ServerMsg, Topic};
use shared::AuthUser;
use uuid::Uuid;

use crate::hub::ConnId;
use crate::state::AppState;

pub async fn can_subscribe(state: &AppState, user: Option<&AuthUser>, topic: &Topic) -> bool {
    match topic {
        Topic::Chat(id) => crate::chat::can_subscribe(&state.db, user, *id).await,
        _ => false,
    }
}

pub async fn on_join(_state: &AppState, _conn: ConnId, _user: Option<&AuthUser>, _topic: &Topic) {}

pub async fn on_leave(_state: &AppState, _conn: ConnId, _user: Option<&AuthUser>, _topic: &Topic) {}

pub async fn on_offline(_state: &AppState, _user_id: Uuid) {}

pub async fn dispatch(state: &AppState, conn: ConnId, user: Option<&AuthUser>, msg: ClientMsg) {
    match msg {
        ClientMsg::Typing { channel_id } => {
            let Some(user) = user else { return };
            if !state.limiter.check(&format!("typing:{}", user.id), 6, Duration::from_secs(10)) {
                return;
            }
            let topic = Topic::Chat(channel_id);
            if !state.hub.user_in_topic(user.id, &topic) {
                return;
            }
            state.hub.publish_except(
                &topic,
                &ServerMsg::ChatTyping {
                    channel_id,
                    user: user.summary(),
                },
                conn,
            );
        }
        _ => {}
    }
}
