//! Chat header actions contributed by other modules (e.g. start a call).

use leptos::prelude::*;
use shared::chat::{ChannelDetail, ChannelKind};
use uuid::Uuid;

pub fn header_actions(channel_id: Uuid, detail: RwSignal<Option<ChannelDetail>>) -> impl IntoView {
    let _ = channel_id;
    // Direct messages get call buttons for the other person.
    let partner = move || {
        detail.with(|d| {
            d.as_ref()
                .filter(|d| d.channel.kind == ChannelKind::Direct)
                .and_then(|d| d.channel.dm_user.as_ref().map(|u| u.id))
        })
    };
    move || partner().map(|user_id| view! { <crate::pages::calls::CallButtons user_id=user_id compact=true/> })
}
