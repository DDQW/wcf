//! Chat header actions contributed by other modules (e.g. start a call).

use leptos::prelude::*;
use shared::chat::ChannelDetail;
use uuid::Uuid;

pub fn header_actions(channel_id: Uuid, detail: RwSignal<Option<ChannelDetail>>) -> impl IntoView {
    let _ = (channel_id, detail);
}
