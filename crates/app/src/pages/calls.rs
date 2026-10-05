//! Calls: room directory and history, room page, 1:1 call page, the media
//! stage (bridged to livekit-client via `assets/js/wcf-call.js`) and the
//! global incoming-call overlay.

use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::{use_navigate, use_params_map};
use serde::Deserialize;
use shared::call::{
    CallEvent, CallRoomDetail, CallRoomInput, CallRoomView, CallStatus, DirectCallView, JoinInfo, RoomVisibility,
};
use shared::chat::ChatMessageView;
use shared::ws::{ServerMsg, Topic};
use shared::UserSummary;
use uuid::Uuid;

use crate::api::calls::*;
use crate::components::common::confirm;
use crate::components::user_picker::UserPicker;
use crate::components::{Avatar, EmptyState, ErrorBox, Icon, Modal, RequireLogin, Spinner, TimeAgo};
use crate::pages::not_found::Missing;
use crate::realtime::use_realtime;
use crate::state::{use_session, use_toasts};

// ---------------------------------------------------------------------------
// JavaScript bridge
// ---------------------------------------------------------------------------

mod js {
    use serde_json::Value;

    /// Calls `window.WcfCall[method](...args)` and awaits a returned promise.
    #[cfg(feature = "hydrate")]
    pub async fn call(method: &str, args: Vec<Value>) -> Result<wasm_bindgen::JsValue, String> {
        use wasm_bindgen::JsCast;
        let win = web_sys::window().ok_or("kein Fenster")?;
        let obj = js_sys::Reflect::get(&win, &"WcfCall".into()).map_err(|_| "WcfCall fehlt")?;
        if obj.is_undefined() {
            return Err("Das Call-Skript wurde nicht geladen.".into());
        }
        let f: js_sys::Function = js_sys::Reflect::get(&obj, &method.into())
            .ok()
            .and_then(|f| f.dyn_into().ok())
            .ok_or("Unbekannte Call-Funktion")?;
        let list = js_sys::Array::new();
        for a in args {
            list.push(&js_sys::JSON::parse(&a.to_string()).unwrap_or(wasm_bindgen::JsValue::NULL));
        }
        let res = f.apply(&obj, &list).map_err(|e| format!("{e:?}"))?;
        match res.dyn_into::<js_sys::Promise>() {
            Ok(p) => wasm_bindgen_futures::JsFuture::from(p).await.map_err(|e| {
                js_sys::Reflect::get(&e, &"message".into())
                    .ok()
                    .and_then(|m| m.as_string())
                    .unwrap_or_else(|| "Fehler".into())
            }),
            Err(v) => Ok(v),
        }
    }

    /// Fire-and-forget variant.
    pub fn fire(method: &'static str, args: Vec<Value>) {
        #[cfg(feature = "hydrate")]
        leptos::task::spawn_local(async move {
            let _ = call(method, args).await;
        });
        #[cfg(not(feature = "hydrate"))]
        let _ = (method, args);
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct JsParticipant {
    identity: String,
    name: String,
    speaking: bool,
    mic: bool,
    cam: bool,
    screen: bool,
    local: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
struct Device {
    id: String,
    label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
struct Devices {
    #[serde(default)]
    audioinput: Vec<Device>,
    #[serde(default)]
    videoinput: Vec<Device>,
    #[serde(default)]
    audiooutput: Vec<Device>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum JsEvent {
    State {
        state: String,
        #[serde(default)]
        error: Option<String>,
    },
    Participants {
        participants: Vec<JsParticipant>,
    },
    Local {
        mic: bool,
        cam: bool,
        screen: bool,
    },
    Audio {
        blocked: bool,
    },
    Devices {
        devices: Devices,
    },
    Error {
        message: String,
    },
}

/// Listens to "wcf-call" window events for the current owner's lifetime.
fn on_js_event(handler: impl Fn(JsEvent) + 'static) {
    #[cfg(feature = "hydrate")]
    {
        use wasm_bindgen::JsCast;
        let h = window_event_listener_untyped("wcf-call", move |ev| {
            if let Some(ce) = ev.dyn_ref::<web_sys::CustomEvent>() {
                if let Some(s) = ce.detail().as_string() {
                    if let Ok(e) = serde_json::from_str::<JsEvent>(&s) {
                        handler(e);
                    }
                }
            }
        });
        on_cleanup(move || h.remove());
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = handler;
}

fn call_event(msg: &ServerMsg) -> Option<CallEvent> {
    match msg {
        ServerMsg::Module { module, payload } => CallEvent::from_payload(module, payload),
        _ => None,
    }
}

fn format_duration(secs: i64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

// ---------------------------------------------------------------------------
// Directory + history
// ---------------------------------------------------------------------------

#[component]
pub fn CallsPage() -> impl IntoView {
    view! {
        <Title text="Anrufe"/>
        <RequireLogin>
            <CallsOverviewView/>
        </RequireLogin>
    }
}

#[component]
fn CallsOverviewView() -> impl IntoView {
    let session = use_session();
    let navigate = use_navigate();
    let version = RwSignal::new(0u32);
    let overview = Resource::new(move || version.get(), |_| fetch_calls_overview());
    let create_open = RwSignal::new(false);
    // Participant counts change constantly; refresh periodically.
    #[cfg(feature = "hydrate")]
    {
        let handle = set_interval_with_handle(move || version.update(|v| *v += 1), std::time::Duration::from_secs(20)).ok();
        on_cleanup(move || {
            if let Some(h) = handle {
                h.clear();
            }
        });
    }
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    let on_created = Callback::new(move |r: CallRoomView| {
        create_open.set(false);
        go.run(r.url());
    });
    let me = move || session.user_id();

    view! {
        <div class="page-header">
            <div>
                <h1>"Anrufe"</h1>
                <p class="muted">"Sprach- und Videoräume, Bildschirmfreigabe und Direktanrufe"</p>
            </div>
            <div class="page-actions">
                <Transition>
                    {move || Suspend::new(async move {
                        overview.await.ok().filter(|o| o.can_create && o.configured).map(|_| view! {
                            <button class="btn btn-primary" on:click=move |_| create_open.set(true)>
                                <Icon name="plus" size=16/>"Neuer Raum"
                            </button>
                        })
                    })}
                </Transition>
            </div>
        </div>
        <Transition fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match overview.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(o) => view! {
                        {(!o.configured).then(|| view! {
                            <div class="alert">
                                <Icon name="alert" size=16/>
                                "Auf diesem Server ist noch kein Medienserver (LiveKit) eingerichtet. Räume können angelegt werden, "
                                "Gespräche sind aber erst nach der Einrichtung möglich."
                            </div>
                        })}
                        <section class="card">
                            <div class="card-head"><h2><Icon name="video"/>"Räume"</h2></div>
                            {if o.rooms.is_empty() {
                                view! { <EmptyState icon="video" title="Noch keine Räume"/> }.into_any()
                            } else {
                                view! {
                                    <div class="room-list">
                                        {o.rooms.into_iter().map(|r| view! { <RoomCard room=r/> }).collect_view()}
                                    </div>
                                }.into_any()
                            }}
                        </section>
                        <section class="card">
                            <div class="card-head"><h2><Icon name="phone"/>"Letzte Anrufe"</h2></div>
                            {if o.history.is_empty() {
                                view! { <p class="muted">"Noch keine Direktanrufe."</p> }.into_any()
                            } else {
                                view! {
                                    <ul class="plain-list call-history">
                                        {o.history.into_iter().map(|c| view! { <HistoryRow call=c me=me().unwrap_or_default() configured=o.configured/> }).collect_view()}
                                    </ul>
                                }.into_any()
                            }}
                        </section>
                    }.into_any(),
                }
            })}
        </Transition>
        <Modal open=create_open title="Neuer Raum">
            <RoomForm room=None on_saved=on_created/>
        </Modal>
    }
}

#[component]
fn RoomCard(room: CallRoomView) -> impl IntoView {
    let count = room.participants.len();
    let live = count > 0;
    let icon = if live { "radio" } else { "video" };
    view! {
        <a class="room-card" class:live=live href=room.url()>
            <div class="room-card-icon"><Icon name=icon size=22/></div>
            <div class="grow">
                <div class="row gap-sm">
                    <strong>{room.name.clone()}</strong>
                    {(room.visibility == RoomVisibility::Private).then(|| view! { <span title="Privat"><Icon name="lock" size=13/></span> })}
                    {(!room.persistent).then(|| view! { <span class="tag-chip">"spontan"</span> })}
                </div>
                {(!room.description.is_empty()).then(|| view! { <div class="muted small ellipsis">{room.description.clone()}</div> })}
            </div>
            {if live {
                view! {
                    <div class="room-live">
                        <div class="avatar-stack">
                            {room.participants.iter().take(5).map(|u| view! { <Avatar user=u.clone() size=24/> }).collect_view()}
                        </div>
                        <span class="live-badge">{format!("{count} live")}</span>
                    </div>
                }.into_any()
            } else {
                view! { <span class="muted small">"aktiv " <TimeAgo ts=room.last_active_at/></span> }.into_any()
            }}
        </a>
    }
}

#[component]
fn HistoryRow(call: DirectCallView, me: Uuid, configured: bool) -> impl IntoView {
    let toasts = use_toasts();
    let navigate = use_navigate();
    let outgoing = call.caller.id == me;
    let other = call.other(me).clone();
    let other_id = other.id;
    let video = call.video;
    let missed = !outgoing && matches!(call.status, CallStatus::Missed | CallStatus::Cancelled);
    let icon = if missed { "phone-off" } else if outgoing { "arrow-up" } else { "arrow-down" };
    let detail = match call.duration_secs() {
        Some(d) => format!("{} · {}", call.status.label(), format_duration(d)),
        None => call.status.label().to_string(),
    };
    let call_back = move |_| {
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match start_direct_call(other_id, video).await {
                Ok(d) => navigate(&d.call.url(), Default::default()),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <li class="call-row" class:missed=missed>
            <Icon name=icon size=16/>
            <Avatar user=other.clone() size=32/>
            <div class="grow">
                <a class="user-link" href=other.profile_url()>{other.display_name.clone()}</a>
                <div class="muted small">
                    {if video { "Videoanruf" } else { "Sprachanruf" }} " · " {detail} " · " <TimeAgo ts=call.created_at/>
                </div>
            </div>
            {configured.then(|| view! {
                <button class="btn btn-sm" on:click=call_back title="Zurückrufen"><Icon name="phone" size=14/>"Anrufen"</button>
            })}
        </li>
    }
}

#[component]
fn RoomForm(room: Option<CallRoomView>, on_saved: Callback<CallRoomView>) -> impl IntoView {
    let toasts = use_toasts();
    let id = room.as_ref().map(|r| r.id);
    let name = RwSignal::new(room.as_ref().map(|r| r.name.clone()).unwrap_or_default());
    let description = RwSignal::new(room.as_ref().map(|r| r.description.clone()).unwrap_or_default());
    let visibility = RwSignal::new(room.as_ref().map(|r| r.visibility).unwrap_or_default());
    let persistent = RwSignal::new(room.as_ref().map(|r| r.persistent).unwrap_or(true));
    let saving = RwSignal::new(false);
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let input = CallRoomInput {
            name: name.get_untracked(),
            description: description.get_untracked(),
            visibility: visibility.get_untracked(),
            persistent: persistent.get_untracked(),
        };
        saving.set(true);
        leptos::task::spawn_local(async move {
            let res = match id {
                Some(id) => update_call_room(id, input).await,
                None => create_call_room(input).await,
            };
            saving.set(false);
            match res {
                Ok(r) => on_saved.run(r),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <form class="form" on:submit=submit>
            <label class="field"><span>"Name"</span>
                <input type="text" maxlength="80" required bind:value=name placeholder="z. B. Spieleabend"/>
            </label>
            <label class="field"><span>"Beschreibung"</span>
                <textarea rows="2" maxlength="1000" bind:value=description></textarea>
            </label>
            <label class="field"><span>"Zugang"</span>
                <select prop:value=move || visibility.get().as_str().to_string()
                    on:change=move |ev| visibility.set(RoomVisibility::parse(&event_target_value(&ev)))>
                    <option value="members">"Alle Mitglieder"</option>
                    <option value="private">"Nur Eingeladene"</option>
                </select>
            </label>
            <label class="check">
                <input type="checkbox" bind:checked=persistent/>
                <span>"Dauerhaft – sonst wird der Raum entfernt, wenn er eine Weile leer ist"</span>
            </label>
            <div class="form-actions">
                <button class="btn btn-primary" type="submit" disabled=move || saving.get()>
                    {if id.is_some() { "Speichern" } else { "Raum anlegen" }}
                </button>
            </div>
        </form>
    }
}

// ---------------------------------------------------------------------------
// Room page
// ---------------------------------------------------------------------------

#[component]
pub fn CallRoomPage() -> impl IntoView {
    let params = use_params_map();
    let room_id = Memo::new(move |_| params.with(|p| p.get("id").and_then(|s| Uuid::parse_str(&s).ok())));
    let version = RwSignal::new(0u32);
    let detail = Resource::new(
        move || (room_id.get(), version.get()),
        |(id, _)| async move {
            match id {
                Some(id) => get_call_room(id).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let reload = Callback::new(move |_| version.update(|v| *v += 1));
    view! {
        <RequireLogin>
            <Transition fallback=|| view! { <Spinner/> }>
                {move || Suspend::new(async move {
                    match detail.await {
                        Ok(None) => view! { <Missing text="Diesen Raum gibt es nicht."/> }.into_any(),
                        Err(e) if e.is_not_found() => view! { <Missing text="Diesen Raum gibt es nicht (mehr) oder du hast keinen Zugang."/> }.into_any(),
                        Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                        Ok(Some(d)) => view! { <RoomBody detail=d reload=reload/> }.into_any(),
                    }
                })}
            </Transition>
        </RequireLogin>
    }
}

#[component]
fn RoomBody(detail: CallRoomDetail, reload: Callback<()>) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let rt = use_realtime();
    let navigate = use_navigate();
    let room = detail.room.clone();
    let room_id = room.id;
    let joined = RwSignal::new(None::<JoinInfo>);
    let joining = RwSignal::new(false);
    let start_mic = RwSignal::new(true);
    let start_cam = RwSignal::new(false);
    let settings_open = RwSignal::new(false);
    let participants = RwSignal::new(room.participants.clone());
    let configured = session.site.with_untracked(|s| s.calls_configured);
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));

    rt.on_message(move |msg| match call_event(msg) {
        Some(CallEvent::Participants { room_id: id, participants: list }) if id == room_id => participants.set(list),
        Some(CallEvent::RoomUpdated { room_id: id }) if id == room_id && joined.get_untracked().is_none() => reload.run(()),
        Some(CallEvent::RoomDeleted { room_id: id }) if id == room_id => {
            joined.set(None);
            toasts.info("Der Raum wurde gelöscht.", None);
            go.run("/calls".into());
        }
        _ => {}
    });

    let join = move |_| {
        joining.set(true);
        leptos::task::spawn_local(async move {
            match join_call_room(room_id).await {
                Ok(info) => {
                    js::fire("preview", vec!["prejoin-video".into(), false.into()]);
                    joined.set(Some(info));
                }
                Err(e) => toasts.error(e.to_string()),
            }
            joining.set(false);
        });
    };
    let toggle_preview = move |_| {
        start_cam.update(|c| *c = !*c);
        js::fire("preview", vec!["prejoin-video".into(), start_cam.get_untracked().into()]);
    };
    on_cleanup(|| js::fire("preview", vec!["prejoin-video".into(), false.into()]));
    let on_leave = Callback::new(move |_| joined.set(None));
    let on_saved = Callback::new(move |_r: CallRoomView| {
        settings_open.set(false);
        reload.run(());
    });
    let remove = move |_| {
        if !confirm("Diesen Raum endgültig löschen?") {
            return;
        }
        leptos::task::spawn_local(async move {
            match delete_call_room(room_id).await {
                Ok(()) => go.run("/calls".into()),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let room_for_form = room.clone();
    let stage_title = StoredValue::new(room.name.clone());
    let detail_members = detail.members.clone();
    let chat_channel = detail.chat_channel_id;
    let can_edit = room.can_edit;
    let can_moderate = room.can_moderate;
    let is_private = room.visibility == RoomVisibility::Private;

    view! {
        <Title text=room.name.clone()/>
        <Show
            when=move || joined.get().is_some()
            fallback=move || {
                let room = room.clone();
                view! {
                    <div class="breadcrumbs"><a href="/calls">"Anrufe"</a><Icon name="chevron-right" size=14/></div>
                    <div class="page-header">
                        <div>
                            <h1>{room.name.clone()}</h1>
                            <div class="muted small row gap-sm wrap">
                                <Icon name=if room.visibility == RoomVisibility::Private { "lock" } else { "users" } size=13/>
                                {room.visibility.label()}
                                " · " {if room.persistent { "dauerhaft" } else { "spontan" }}
                                " · erstellt von " {room.owner.display_name.clone()}
                            </div>
                        </div>
                        <div class="page-actions">
                            {can_edit.then(|| view! {
                                <button class="btn" on:click=move |_| settings_open.set(true)><Icon name="edit" size=16/>"Bearbeiten"</button>
                                <button class="btn btn-danger-soft" on:click=remove title="Raum löschen"><Icon name="trash" size=16/></button>
                            })}
                        </div>
                    </div>
                    {(!room.description.is_empty()).then(|| view! { <p class="room-description">{room.description.clone()}</p> })}
                    <div class="prejoin">
                        <div class="prejoin-preview">
                            <video id="prejoin-video" autoplay=true playsinline=true muted=true></video>
                            <Show when=move || !start_cam.get()>
                                <div class="prejoin-off"><Icon name="video-off" size=32/><span>"Kamera aus"</span></div>
                            </Show>
                        </div>
                        <div class="prejoin-side card">
                            <h2>"Bereit zum Beitreten?"</h2>
                            {move || {
                                let list = participants.get();
                                if list.is_empty() {
                                    view! { <p class="muted">"Gerade ist niemand im Raum."</p> }.into_any()
                                } else {
                                    view! {
                                        <div class="row gap-sm wrap">
                                            <div class="avatar-stack">{list.iter().take(8).map(|u| view! { <Avatar user=u.clone() size=28/> }).collect_view()}</div>
                                            <span class="muted small">{format!("{} im Raum", list.len())}</span>
                                        </div>
                                    }.into_any()
                                }
                            }}
                            <div class="row gap-sm">
                                <button class="call-btn" class:off=move || !start_mic.get() title="Mikrofon"
                                    on:click=move |_| start_mic.update(|m| *m = !*m)>
                                    {move || view! { <Icon name=if start_mic.get() { "mic" } else { "mic-off" } size=20/> }}
                                </button>
                                <button class="call-btn" class:off=move || !start_cam.get() title="Kamera" on:click=toggle_preview>
                                    {move || view! { <Icon name=if start_cam.get() { "video" } else { "video-off" } size=20/> }}
                                </button>
                            </div>
                            {if configured {
                                view! {
                                    <button class="btn btn-primary btn-lg" on:click=join disabled=move || joining.get()>
                                        <Icon name="phone" size=18/>{move || if joining.get() { "Verbinde …" } else { "Beitreten" }}
                                    </button>
                                }.into_any()
                            } else {
                                view! { <div class="alert small">"Der Medienserver ist nicht eingerichtet."</div> }.into_any()
                            }}
                        </div>
                    </div>
                }
            }
        >
            {move || joined.get().map(|info| view! {
                <CallStage
                    join=info
                    topic_id=room_id
                    room_id=Some(room_id)
                    title=stage_title.get_value()
                    start_mic=start_mic.get_untracked()
                    start_cam=start_cam.get_untracked()
                    chat_channel=chat_channel
                    on_leave=on_leave
                />
            })}
        </Show>
        <Modal open=settings_open title="Raum bearbeiten" wide=true>
            <RoomForm room=Some(room_for_form.clone()) on_saved=on_saved/>
            {(is_private || can_moderate).then(|| view! {
                <RoomMembers room_id=room_id members=detail_members.clone() can_appoint=can_edit reload=reload/>
            })}
        </Modal>
    }
}

#[component]
fn RoomMembers(room_id: Uuid, members: Vec<shared::call::RoomMember>, can_appoint: bool, reload: Callback<()>) -> impl IntoView {
    let toasts = use_toasts();
    let picked = RwSignal::new(Vec::<UserSummary>::new());
    let set = move |user_id: Uuid, role: Option<bool>| {
        leptos::task::spawn_local(async move {
            match set_call_room_member(room_id, user_id, role).await {
                Ok(()) => reload.run(()),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let add = move |_| {
        for u in picked.get_untracked() {
            set(u.id, Some(false));
        }
        picked.set(Vec::new());
    };
    view! {
        <div class="room-members">
            <h3>"Eingeladene & Moderatoren"</h3>
            <p class="muted small">"In privaten Räumen haben nur eingeladene Mitglieder Zugang. Moderatoren dürfen Teilnehmer stummschalten und entfernen."</p>
            <ul class="plain-list">
                {members.into_iter().map(|m| {
                    let uid = m.user.id;
                    let is_mod = m.moderator;
                    view! {
                        <li class="row gap-sm member-row">
                            <Avatar user=m.user.clone() size=28/>
                            <span class="grow">{m.user.display_name.clone()}</span>
                            {is_mod.then(|| view! { <span class="tag-chip">"Moderator"</span> })}
                            {can_appoint.then(|| view! {
                                <button class="btn btn-sm btn-ghost" on:click=move |_| set(uid, Some(!is_mod))>
                                    {if is_mod { "Moderator entziehen" } else { "Zum Moderator" }}
                                </button>
                            })}
                            <button class="btn btn-sm btn-ghost" title="Entfernen" on:click=move |_| set(uid, None)><Icon name="x" size=14/></button>
                        </li>
                    }
                }).collect_view()}
            </ul>
            <div class="row gap-sm">
                <div class="grow"><UserPicker selected=picked placeholder="Mitglied einladen …"/></div>
                <button class="btn" on:click=add disabled=move || picked.with(|p| p.is_empty())>"Einladen"</button>
            </div>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Media stage
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    People,
    Chat,
    Devices,
}

#[component]
fn CallStage(
    join: JoinInfo,
    /// Realtime topic id (room or 1:1 call) used for presence.
    topic_id: Uuid,
    /// Room for moderation actions (`None` for 1:1 calls).
    room_id: Option<Uuid>,
    #[prop(into)] title: String,
    start_mic: bool,
    start_cam: bool,
    chat_channel: Option<Uuid>,
    on_leave: Callback<()>,
) -> impl IntoView {
    let toasts = use_toasts();
    let rt = use_realtime();
    let session = use_session();
    let state = RwSignal::new("connecting".to_string());
    let error = RwSignal::new(None::<String>);
    let media = RwSignal::new(Vec::<JsParticipant>::new());
    let present = RwSignal::new(Vec::<UserSummary>::new());
    let mic = RwSignal::new(false);
    let cam = RwSignal::new(false);
    let screen = RwSignal::new(false);
    let audio_blocked = RwSignal::new(false);
    let devices = RwSignal::new(Devices::default());
    let panel = RwSignal::new(None::<Panel>);
    let moderator = join.moderator;
    let me = session.user_id();

    rt.subscribe_scoped(Topic::Call(topic_id));
    rt.on_message(move |msg| match call_event(msg) {
        Some(CallEvent::Participants { room_id: id, participants }) if id == topic_id => present.set(participants),
        Some(CallEvent::MuteRequest { room_id: id, by }) if Some(id) == room_id => {
            if mic.get_untracked() {
                js::fire("toggle", vec!["mic".into()]);
            }
            toasts.info(format!("{} hat dich stummgeschaltet.", by.display_name), None);
        }
        Some(CallEvent::Removed { room_id: id, by }) if Some(id) == room_id => {
            js::fire("leave", vec![]);
            toasts.error(format!("{} hat dich aus dem Raum entfernt.", by.display_name));
            on_leave.run(());
        }
        _ => {}
    });

    on_js_event(move |ev| match ev {
        JsEvent::State { state: s, error: e } => {
            if s == "failed" {
                error.set(Some(e.unwrap_or_else(|| "Verbindung fehlgeschlagen.".into())));
            } else if s == "connected" {
                error.set(None);
            }
            state.set(s);
        }
        JsEvent::Participants { participants } => media.set(participants),
        JsEvent::Local { mic: m, cam: c, screen: s } => {
            mic.set(m);
            cam.set(c);
            screen.set(s);
        }
        JsEvent::Audio { blocked } => audio_blocked.set(blocked),
        JsEvent::Devices { devices: d } => devices.set(d),
        JsEvent::Error { message } => toasts.error(message),
    });

    let opts = serde_json::json!({
        "url": join.url,
        "token": join.token,
        "identity": join.identity,
        "containerId": "call-grid",
        "mic": start_mic,
        "cam": start_cam,
    });
    let connect = move || {
        let opts = opts.clone();
        state.set("connecting".into());
        error.set(None);
        #[cfg(feature = "hydrate")]
        leptos::task::spawn_local(async move {
            if let Err(e) = js::call("join", vec![opts]).await {
                error.set(Some(e));
                state.set("failed".into());
            }
        });
        #[cfg(not(feature = "hydrate"))]
        let _ = opts;
    };
    let connect_again = connect.clone();
    Effect::new(move |prev: Option<()>| {
        if prev.is_none() {
            connect();
        }
    });
    on_cleanup(|| js::fire("leave", vec![]));

    let leave = move |_| {
        js::fire("leave", vec![]);
        on_leave.run(());
    };
    let open_devices = move |_| {
        panel.update(|p| *p = if *p == Some(Panel::Devices) { None } else { Some(Panel::Devices) });
        #[cfg(feature = "hydrate")]
        leptos::task::spawn_local(async move {
            if let Ok(v) = js::call("devices", vec![]).await {
                if let Some(s) = js_sys::JSON::stringify(&v).ok().and_then(|s| s.as_string()) {
                    if let Ok(d) = serde_json::from_str::<Devices>(&s) {
                        devices.set(d);
                    }
                }
            }
        });
    };
    let toggle_panel = move |p: Panel| panel.update(|cur| *cur = if *cur == Some(p) { None } else { Some(p) });
    let count = move || media.with(|m| m.len()).max(present.with(|p| p.len()));

    view! {
        <div class="call-stage" class:with-panel=move || panel.get().is_some()>
            <div class="call-main">
                <div class="call-top">
                    <strong class="ellipsis">{title}</strong>
                    <span class="muted small">{move || match state.get().as_str() {
                        "connected" => format!("{} Teilnehmer", count()),
                        "reconnecting" => "Verbindung wird wiederhergestellt …".to_string(),
                        "failed" => "Nicht verbunden".to_string(),
                        "disconnected" => "Getrennt".to_string(),
                        _ => "Verbinde …".to_string(),
                    }}</span>
                </div>
                <Show when=move || audio_blocked.get()>
                    <button class="call-audio-banner" on:click=move |_| js::fire("startAudio", vec![])>
                        <Icon name="volume-x" size=16/>"Der Browser blockiert den Ton – hier klicken, um ihn zu aktivieren."
                    </button>
                </Show>
                <div class="call-grid-wrap">
                    <div id="call-grid" class="call-grid"></div>
                    <Show when=move || error.get().is_some()>
                        <div class="call-overlay">
                            <Icon name="alert" size=32/>
                            <p>{move || error.get().unwrap_or_default()}</p>
                            <button class="btn" on:click={
                                let connect_again = connect_again.clone();
                                move |_| connect_again()
                            }>"Erneut versuchen"</button>
                        </div>
                    </Show>
                    <Show when=move || state.get() == "connecting" && error.get().is_none()>
                        <div class="call-overlay"><Spinner label="Verbinde mit dem Medienserver …"/></div>
                    </Show>
                </div>
                <div class="call-controls">
                    <button class="call-btn" class:off=move || !mic.get() title="Mikrofon (an/aus)" on:click=move |_| js::fire("toggle", vec!["mic".into()])>
                        {move || view! { <Icon name=if mic.get() { "mic" } else { "mic-off" } size=20/> }}
                    </button>
                    <button class="call-btn" class:off=move || !cam.get() title="Kamera (an/aus)" on:click=move |_| js::fire("toggle", vec!["cam".into()])>
                        {move || view! { <Icon name=if cam.get() { "video" } else { "video-off" } size=20/> }}
                    </button>
                    <button class="call-btn" class:active=move || screen.get() title="Bildschirm teilen" on:click=move |_| js::fire("toggle", vec!["screen".into()])>
                        <Icon name="monitor" size=20/>
                    </button>
                    <button class="call-btn" class:active=move || panel.get() == Some(Panel::Devices) title="Geräte" on:click=open_devices>
                        <Icon name="sliders" size=20/>
                    </button>
                    <button class="call-btn" class:active=move || panel.get() == Some(Panel::People) title="Teilnehmer" on:click=move |_| toggle_panel(Panel::People)>
                        <Icon name="users" size=20/><span class="call-btn-count">{count}</span>
                    </button>
                    {chat_channel.map(|_| view! {
                        <button class="call-btn" class:active=move || panel.get() == Some(Panel::Chat) title="Chat" on:click=move |_| toggle_panel(Panel::Chat)>
                            <Icon name="message-circle" size=20/>
                        </button>
                    })}
                    <button class="call-btn hangup" title="Verlassen" on:click=leave>
                        <Icon name="phone-off" size=20/>
                    </button>
                </div>
            </div>
            <Show when=move || panel.get().is_some()>
                <aside class="call-panel">
                    <div class="call-panel-head">
                        <strong>{move || match panel.get() {
                            Some(Panel::People) => "Teilnehmer",
                            Some(Panel::Chat) => "Chat",
                            _ => "Geräte",
                        }}</strong>
                        <button class="btn btn-sm btn-ghost" on:click=move |_| panel.set(None)><Icon name="x" size=16/></button>
                    </div>
                    {move || match panel.get() {
                        Some(Panel::People) => view! {
                            <PeoplePanel present=present media=media room_id=room_id moderator=moderator me=me/>
                        }.into_any(),
                        Some(Panel::Chat) => match chat_channel {
                            Some(id) => view! { <CallChat channel_id=id/> }.into_any(),
                            None => ().into_any(),
                        },
                        _ => view! { <DevicesPanel devices=devices/> }.into_any(),
                    }}
                </aside>
            </Show>
        </div>
    }
}

#[component]
fn PeoplePanel(
    present: RwSignal<Vec<UserSummary>>,
    media: RwSignal<Vec<JsParticipant>>,
    room_id: Option<Uuid>,
    moderator: bool,
    me: Option<Uuid>,
) -> impl IntoView {
    let toasts = use_toasts();
    let act = move |user_id: Uuid, remove: bool| {
        let Some(room) = room_id else { return };
        if remove && !confirm("Teilnehmer aus dem Raum entfernen?") {
            return;
        }
        leptos::task::spawn_local(async move {
            let res = if remove {
                remove_call_participant(room, user_id).await
            } else {
                mute_call_participant(room, user_id).await
            };
            match res {
                Ok(()) => toasts.success(if remove { "Teilnehmer entfernt." } else { "Stummgeschaltet." }),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <ul class="plain-list people-list">
            {move || present.get().into_iter().map(|u| {
                let id_str = u.id.to_string();
                let info = media.with(|m| m.iter().find(|p| p.identity == id_str).cloned());
                let uid = u.id;
                let is_me = Some(uid) == me;
                view! {
                    <li class="row gap-sm" class:speaking=info.as_ref().is_some_and(|i| i.speaking)>
                        <Avatar user=u.clone() size=30/>
                        <span class="grow ellipsis">{u.display_name.clone()}{is_me.then_some(" (du)")}</span>
                        {match &info {
                            Some(i) => view! {
                                <span class="muted" title=if i.mic { "Mikrofon an" } else { "Mikrofon aus" }>
                                    <Icon name=if i.mic { "mic" } else { "mic-off" } size=14/>
                                </span>
                                {i.cam.then(|| view! { <span class="muted" title="Kamera an"><Icon name="video" size=14/></span> })}
                                {i.screen.then(|| view! { <span class="muted" title="Teilt den Bildschirm"><Icon name="monitor" size=14/></span> })}
                            }.into_any(),
                            None => view! { <span class="muted small">"ohne Medien"</span> }.into_any(),
                        }}
                        {(moderator && room_id.is_some() && !is_me).then(|| view! {
                            <button class="btn btn-sm btn-ghost" title="Stummschalten" on:click=move |_| act(uid, false)><Icon name="volume-x" size=14/></button>
                            <button class="btn btn-sm btn-ghost" title="Entfernen" on:click=move |_| act(uid, true)><Icon name="user-x" size=14/></button>
                        })}
                    </li>
                }
            }).collect_view()}
        </ul>
    }
}

#[component]
fn DevicesPanel(devices: RwSignal<Devices>) -> impl IntoView {
    let select = move |kind: &'static str, label: &'static str, list: Vec<Device>| {
        view! {
            <label class="field">
                <span>{label}</span>
                <select on:change=move |ev| js::fire("setDevice", vec![kind.into(), event_target_value(&ev).into()])>
                    {list.into_iter().map(|d| view! { <option value=d.id.clone()>{d.label.clone()}</option> }).collect_view()}
                </select>
            </label>
        }
    };
    view! {
        <div class="form devices-panel">
            {move || {
                let d = devices.get();
                view! {
                    {select("audioinput", "Mikrofon", d.audioinput)}
                    {select("videoinput", "Kamera", d.videoinput)}
                    {(!d.audiooutput.is_empty()).then(|| select("audiooutput", "Lautsprecher", d.audiooutput.clone()))}
                }
            }}
            <p class="muted small">"Gerätenamen erscheinen, sobald der Browser Zugriff auf Kamera bzw. Mikrofon hat."</p>
        </div>
    }
}

#[component]
fn CallChat(channel_id: Uuid) -> impl IntoView {
    let toasts = use_toasts();
    let rt = use_realtime();
    let messages = RwSignal::new(Vec::<ChatMessageView>::new());
    let body = RwSignal::new(String::new());
    let list_ref = NodeRef::<leptos::html::Div>::new();
    let scroll_down = move || {
        if let Some(el) = list_ref.get_untracked() {
            el.set_scroll_top(el.scroll_height());
        }
    };
    rt.subscribe_scoped(Topic::Chat(channel_id));
    rt.on_message(move |msg| match msg {
        ServerMsg::ChatMessage { message } if message.channel_id == channel_id => {
            let m = message.clone();
            messages.update(|v| {
                if !v.iter().any(|x| x.id == m.id) {
                    v.push(m);
                }
            });
            scroll_down();
        }
        ServerMsg::ChatMessageDeleted { channel_id: c, message_id } if *c == channel_id => {
            let id = *message_id;
            messages.update(|v| v.retain(|m| m.id != id));
        }
        _ => {}
    });
    Effect::new(move |prev: Option<()>| {
        if prev.is_none() {
            leptos::task::spawn_local(async move {
                if let Ok(page) = crate::api::chat::list_messages(channel_id, None, None, None).await {
                    messages.set(page.messages);
                    scroll_down();
                }
            });
        }
    });
    let send = move || {
        let text = body.get_untracked();
        if text.trim().is_empty() {
            return;
        }
        body.set(String::new());
        leptos::task::spawn_local(async move {
            match crate::api::chat::send_message(channel_id, text, None, Vec::new()).await {
                Ok(m) => {
                    messages.update(|v| {
                        if !v.iter().any(|x| x.id == m.id) {
                            v.push(m);
                        }
                    });
                    scroll_down();
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <div class="call-chat">
            <div class="call-chat-list" node_ref=list_ref>
                <Show when=move || !messages.with(|m| m.is_empty())
                    fallback=|| view! { <p class="muted small center">"Noch keine Nachrichten."</p> }>
                    <For each=move || messages.get() key=|m| m.id children=|m| view! {
                        <div class="call-chat-msg">
                            {m.author.clone().map(|a| view! { <Avatar user=a.clone() size=22/><strong>{a.display_name.clone()}</strong> })}
                            <span class="muted small"><TimeAgo ts=m.created_at/></span>
                            <div class="prose small" inner_html=m.body_html.clone()></div>
                        </div>
                    }/>
                </Show>
            </div>
            <form class="call-chat-form" on:submit=move |ev| { ev.prevent_default(); send(); }>
                <textarea rows="2" placeholder="Nachricht …" bind:value=body
                    on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                        if ev.key() == "Enter" && !ev.shift_key() {
                            ev.prevent_default();
                            send();
                        }
                    }></textarea>
                <button class="btn btn-primary btn-sm" type="submit"><Icon name="send" size=14/></button>
            </form>
        </div>
    }
}

// ---------------------------------------------------------------------------
// 1:1 call page
// ---------------------------------------------------------------------------

#[component]
pub fn DirectCallPage() -> impl IntoView {
    let params = use_params_map();
    let call_id = Memo::new(move |_| params.with(|p| p.get("id").and_then(|s| Uuid::parse_str(&s).ok())));
    let version = RwSignal::new(0u32);
    let detail = Resource::new(
        move || (call_id.get(), version.get()),
        |(id, _)| async move {
            match id {
                Some(id) => get_direct_call(id).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let rt = use_realtime();
    rt.on_message(move |msg| {
        if let Some(CallEvent::Status { call }) = call_event(msg) {
            if Some(call.id) == call_id.get_untracked() {
                version.update(|v| *v += 1);
            }
        }
    });
    view! {
        <Title text="Anruf"/>
        <RequireLogin>
            <Transition fallback=|| view! { <Spinner/> }>
                {move || Suspend::new(async move {
                    match detail.await {
                        Ok(None) | Err(_) => view! { <Missing text="Diesen Anruf gibt es nicht."/> }.into_any(),
                        Ok(Some(d)) => view! { <DirectCallBody call=d.call join=d.join/> }.into_any(),
                    }
                })}
            </Transition>
        </RequireLogin>
    }
}

#[component]
fn DirectCallBody(call: DirectCallView, join: Option<JoinInfo>) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let navigate = use_navigate();
    let me = session.user_id().unwrap_or_default();
    let other = call.other(me).clone();
    let call_id = call.id;
    let status = call.status;
    let ringing_out = status == CallStatus::Ringing && call.caller.id == me;
    let video = call.video;
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    let hangup = move || {
        leptos::task::spawn_local(async move {
            if let Err(e) = hangup_direct_call(call_id).await {
                toasts.error(e.to_string());
            }
        });
    };
    #[cfg(feature = "hydrate")]
    {
        if ringing_out {
            js::fire("ring", vec![true.into()]);
        }
        on_cleanup(|| js::fire("ring", vec![false.into()]));
    }
    let on_leave = Callback::new(move |_| {
        hangup();
        go.run("/calls".into());
    });
    let other_name = other.display_name.clone();

    view! {
        {match (status, join) {
            (CallStatus::Accepted, Some(info)) | (CallStatus::Ringing, Some(info)) => view! {
                {ringing_out.then(|| view! {
                    <div class="ring-banner">
                        <Avatar user=other.clone() size=28/>
                        <span>{format!("Klingelt bei {} …", other_name)}</span>
                    </div>
                })}
                <CallStage
                    join=info
                    topic_id=call_id
                    room_id=None
                    title=format!("Anruf mit {}", other.display_name)
                    start_mic=true
                    start_cam=video
                    chat_channel=None
                    on_leave=on_leave
                />
            }.into_any(),
            (s, _) => view! {
                <div class="call-ended card">
                    <Avatar user=other.clone() size=72/>
                    <h2>{other.display_name.clone()}</h2>
                    <p class="muted">{match s {
                        CallStatus::Ringing => "Der Anruf klingelt …".to_string(),
                        other_status => format!("Anruf {}", other_status.label().to_lowercase()),
                    }}</p>
                    {call.duration_secs().map(|d| view! { <p class="muted small">{format!("Dauer: {}", format_duration(d))}</p> })}
                    <div class="row gap">
                        <a class="btn" href="/calls">"Zur Übersicht"</a>
                        {(!s.is_open() && session.site.with_untracked(|x| x.calls_configured)).then(|| {
                            let other_id = other.id;
                            view! {
                                <button class="btn btn-primary" on:click=move |_| {
                                    leptos::task::spawn_local(async move {
                                        match start_direct_call(other_id, video).await {
                                            Ok(d) => go.run(d.call.url()),
                                            Err(e) => toasts.error(e.to_string()),
                                        }
                                    });
                                }><Icon name="phone" size=16/>"Erneut anrufen"</button>
                            }
                        })}
                    </div>
                </div>
            }.into_any(),
        }}
    }
}

// ---------------------------------------------------------------------------
// Incoming call overlay (mounted once in the shell)
// ---------------------------------------------------------------------------

#[component]
pub fn IncomingCallHost() -> impl IntoView {
    let session = use_session();
    let rt = use_realtime();
    let toasts = use_toasts();
    let navigate = use_navigate();
    let incoming = RwSignal::new(None::<DirectCallView>);
    let busy = RwSignal::new(false);

    rt.on_message(move |msg| match call_event(msg) {
        Some(CallEvent::Incoming { call }) => {
            incoming.set(Some(call));
            js::fire("ring", vec![true.into()]);
        }
        Some(CallEvent::Status { call })
            if incoming.with_untracked(|c| c.as_ref().is_some_and(|c| c.id == call.id)) && call.status != CallStatus::Ringing => {
                incoming.set(None);
                js::fire("ring", vec![false.into()]);
            }
        _ => {}
    });
    // Calls that started ringing before this page was loaded.
    Effect::new(move |_| {
        let ready = session.logged_in() && session.site.with(|s| s.modules.calls && s.calls_configured);
        if ready {
            leptos::task::spawn_local(async move {
                if let Ok(list) = incoming_calls().await {
                    if let Some(c) = list.into_iter().next() {
                        incoming.set(Some(c));
                        js::fire("ring", vec![true.into()]);
                    }
                }
            });
        }
    });

    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    let respond = move |accept: bool| {
        let Some(call) = incoming.get_untracked() else { return };
        busy.set(true);
        js::fire("ring", vec![false.into()]);
        leptos::task::spawn_local(async move {
            match answer_direct_call(call.id, accept).await {
                Ok(d) => {
                    incoming.set(None);
                    if accept {
                        go.run(d.call.url());
                    }
                }
                Err(e) => {
                    incoming.set(None);
                    toasts.error(e.to_string());
                }
            }
            busy.set(false);
        });
    };

    view! {
        <Show when=move || incoming.get().is_some()>
            {move || incoming.get().map(|c| view! {
                <div class="incoming-call" role="alertdialog" aria-label="Eingehender Anruf">
                    <div class="incoming-pulse"><Avatar user=c.caller.clone() size=64/></div>
                    <div class="incoming-text">
                        <strong>{c.caller.display_name.clone()}</strong>
                        <span class="muted small">{if c.video { "Videoanruf" } else { "Sprachanruf" }}</span>
                    </div>
                    <div class="row gap-sm">
                        <button class="call-btn hangup" title="Ablehnen" disabled=move || busy.get() on:click=move |_| respond(false)>
                            <Icon name="phone-off" size=20/>
                        </button>
                        <button class="call-btn accept" title="Annehmen" disabled=move || busy.get() on:click=move |_| respond(true)>
                            <Icon name=if c.video { "video" } else { "phone" } size=20/>
                        </button>
                    </div>
                </div>
            })}
        </Show>
    }
}

/// "Anrufen" buttons for profiles and direct-message headers.
#[component]
pub fn CallButtons(user_id: Uuid, #[prop(optional)] compact: bool) -> impl IntoView {
    let session = use_session();
    let toasts = use_toasts();
    let navigate = use_navigate();
    let go = Callback::new(move |path: String| navigate(&path, Default::default()));
    let start = move |video: bool| {
        leptos::task::spawn_local(async move {
            match start_direct_call(user_id, video).await {
                Ok(d) => go.run(d.call.url()),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let visible = move || {
        session.logged_in()
            && session.user_id() != Some(user_id)
            && session.site.with(|s| s.modules.calls && s.calls_configured)
    };
    view! {
        <Show when=visible>
            <button class=if compact { "btn btn-sm btn-ghost" } else { "btn" } title="Sprachanruf" on:click=move |_| start(false)>
                <Icon name="phone" size=16/>{(!compact).then_some("Anrufen")}
            </button>
            <button class=if compact { "btn btn-sm btn-ghost" } else { "btn" } title="Videoanruf" on:click=move |_| start(true)>
                <Icon name="video" size=16/>{(!compact).then_some("Video")}
            </button>
        </Show>
    }
}
