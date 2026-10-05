//! Realtime hooks of modules added in later phases (chat typing, watch
//! rooms, call signalling). The core `/ws` loop calls into these.

use shared::ws::{ClientMsg, Topic};
use shared::AuthUser;
use uuid::Uuid;

use crate::hub::ConnId;
use crate::state::AppState;

pub async fn can_subscribe(_state: &AppState, _user: Option<&AuthUser>, _topic: &Topic) -> bool {
    false
}

pub async fn on_join(_state: &AppState, _conn: ConnId, _user: Option<&AuthUser>, _topic: &Topic) {}

pub async fn on_leave(_state: &AppState, _conn: ConnId, _user: Option<&AuthUser>, _topic: &Topic) {}

pub async fn on_offline(_state: &AppState, _user_id: Uuid) {}

pub async fn dispatch(_state: &AppState, _conn: ConnId, _user: Option<&AuthUser>, _msg: ClientMsg) {}
