//! Showmaster console (port of HQTube's ControlRoom): live status with
//! timecode, transport, show overrides, rundown, audience moderation,
//! requests and the show scheduler.

use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_params_map;
use shared::watch::{
    fmt_timecode, ControlAction, EventInfo, EventInput, EventItemInput, EventStatus, MediaInfo, MediaStatus,
    PlaybackState, ProposalInfo, ProposalStatus, ShowInfo,
};
use uuid::Uuid;

use crate::api::watch::*;
use crate::components::common::{confirm, datetime_local_value, local_datetime, parse_datetime_local};
use crate::components::{ErrorBox, Icon, RequireLogin, Spinner};
use crate::pages::not_found::Missing;
use crate::pages::watch::{clock_of, local, ticker, RequestsPanel, RoomConn, Transport, ViewersPanel, WatchChat};
use crate::state::{now_ms, use_toasts};

/// Two-step confirm for irreversible show actions: the first click arms,
/// doing nothing for 3 s disarms (never steals focus mid-show).
#[component]
fn ArmedButton(
    #[prop(into)] label: String,
    #[prop(optional, into)] armed_label: Option<String>,
    on_fire: Callback<()>,
    #[prop(optional)] danger: bool,
) -> impl IntoView {
    let armed = RwSignal::new(false);
    let armed_text = armed_label.unwrap_or_else(|| format!("Sicher? {label}"));
    let click = move |_| {
        if armed.get_untracked() {
            armed.set(false);
            on_fire.run(());
        } else {
            armed.set(true);
            #[cfg(feature = "hydrate")]
            leptos::task::spawn_local(async move {
                crate::components::common::sleep_ms(3000).await;
                armed.set(false);
            });
        }
    };
    view! {
        <button class="btn btn-sm" class:btn-armed=move || armed.get() class:btn-ghost=move || !armed.get() class:danger=danger on:click=click>
            {move || if armed.get() { armed_text.clone() } else { label.clone() }}
        </button>
    }
}

#[component]
pub fn WatchConsolePage() -> impl IntoView {
    let params = use_params_map();
    let slug = Memo::new(move |_| params.with(|p| p.get("slug").unwrap_or_default()));
    let room = Resource::new(move || slug.get(), get_watch_room);
    view! {
        <RequireLogin>
            <Suspense fallback=|| view! { <Spinner/> }>
                {move || Suspend::new(async move {
                    match room.await {
                        Err(e) if e.is_not_found() => view! { <Missing text="Diesen Raum gibt es nicht."/> }.into_any(),
                        Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                        Ok(r) if !r.can_control => view! {
                            <div class="card center">
                                <p class="error">"Die Konsole ist für Inhaber und Moderatoren des Raums."</p>
                                <a class="btn" href=r.url()>"Zurück zum Raum"</a>
                            </div>
                        }.into_any(),
                        Ok(r) => view! { <Console room=r/> }.into_any(),
                    }
                })}
            </Suspense>
        </RequireLogin>
    }
}

#[component]
fn Console(room: shared::watch::WatchRoomView) -> impl IntoView {
    let title = format!("Konsole · {}", room.name);
    let room_url = room.url();
    let conn = RoomConn::new(room);
    let tick = ticker(1000);
    let prefill = RwSignal::new(None::<(u64, String, Option<Uuid>)>);
    let events_version = RwSignal::new(0u32);
    // Show transitions change event rows — refresh the schedule.
    Effect::new(move |_| {
        conn.show.with(|s| s.as_ref().map(|s| (s.event_id, s.status)));
        events_version.update(|v| *v += 1);
    });
    let on_promote = Callback::new(move |p: ProposalInfo| prefill.set(Some((now_ms() as u64, p.title, Some(p.id)))));
    view! {
        <Title text=title/>
        <div class="console">
            <header class="console-head card">
                <a class="btn btn-ghost btn-sm" href=room_url><Icon name="arrow-left" size=14/>"Raum"</a>
                <strong class="ellipsis">{move || conn.room.with(|r| r.as_ref().map(|r| r.name.clone()).unwrap_or_default())}</strong>
                <span class="tag-chip">"Konsole"</span>
                <span class="grow"></span>
                <span class="muted small">{move || format!("{} Zuschauer", conn.presence.with(|p| p.len()))}</span>
                <span class="console-clock mono">{move || { tick.track(); clock_of(conn.server_now() as i64) }}</span>
            </header>
            <div class="console-columns">
                <div class="console-body">
                    <section class="card console-section">
                        <h2>"Jetzt"</h2>
                        <NowPlaying conn=conn/>
                        {move || conn.show.get().map(|s| view! { <ShowControls conn=conn show=s/> })}
                    </section>
                    {move || conn.show.get().map(|s| view! {
                        <section class="card console-section">
                            <h2>{format!("Ablauf – {}", s.title)}</h2>
                            <Rundown show=s/>
                        </section>
                    })}
                    <section class="card console-section">
                        <h2>"Publikum"</h2>
                        <ViewersPanel conn=conn/>
                    </section>
                    <section class="card console-section">
                        <h2>"Wünsche"</h2>
                        <RequestsPanel conn=conn manage=true on_promote=on_promote/>
                    </section>
                    <section class="card console-section">
                        <h2>"Vorstellungen"</h2>
                        <EventScheduler conn=conn version=events_version prefill=prefill/>
                    </section>
                </div>
                <aside class="card console-chat"><WatchChat conn=conn/></aside>
            </div>
        </div>
    }
}

#[component]
fn NowPlaying(conn: RoomConn) -> impl IntoView {
    let tick = ticker(250);
    let now = move || {
        tick.track();
        conn.server_now()
    };
    view! {
        <div class="nowplaying">
            {move || {
                let state = conn.state.get();
                let show = conn.show.get();
                let (badge, cls, title, sub, count): (&str, &str, String, Option<String>, Option<String>) = match &state {
                    PlaybackState::Idle => ("frei", "idle", "Gerade läuft nichts".into(), None, None),
                    PlaybackState::Preshow { title, starts_at_ms, .. } => (
                        "Vorprogramm", "preshow", title.clone(),
                        Some(format!("beginnt um {}", clock_of(*starts_at_ms))),
                        Some(fmt_timecode(*starts_at_ms as f64 - now())),
                    ),
                    PlaybackState::Playing { media, .. } | PlaybackState::Paused { media, .. } => {
                        let playing = matches!(state, PlaybackState::Playing { .. });
                        let sub = show.as_ref().and_then(|s| s.current_index.map(|i| format!("Film {} von {}", i + 1, s.items.len())));
                        (if playing { "läuft" } else { "pausiert" }, if playing { "live" } else { "paused" }, media.title.clone(), sub, None)
                    }
                    PlaybackState::Intermission { ends_at_ms, next_title, item_index, item_count, .. } => (
                        "Pause", "intermission", "Pause".into(),
                        next_title.as_ref().map(|t| format!("als Nächstes: {t} ({} von {})", item_index + 2, item_count)),
                        Some(fmt_timecode(*ends_at_ms as f64 - now())),
                    ),
                    PlaybackState::Ended { title, .. } => ("beendet", "idle", title.clone(), Some("Das war's".into()), None),
                };
                view! {
                    <div class="nowplaying-head">
                        <span class=format!("phase-badge phase-{cls}")>{badge}</span>
                        <strong class="nowplaying-title">{title}</strong>
                        {sub.map(|s| view! { <span class="muted small">{s}</span> })}
                        {count.map(|c| view! { <span class="nowplaying-count mono">{c}</span> })}
                    </div>
                }
            }}
            <Show when=move || conn.state.with(|s| s.media().is_some())>
                <Transport conn=conn/>
                <Show when=move || !conn.show_live()>
                    <div class="row gap-sm">
                        <Show when=move || conn.current_item.get().is_some()>
                            <button class="btn btn-sm" on:click=move |_| conn.control(ControlAction::Next, |_| {})><Icon name="skip-forward" size=14/>"Nächstes"</button>
                        </Show>
                        <button class="btn btn-sm btn-ghost" on:click=move |_| conn.control(ControlAction::Stop, |_| {})><Icon name="x" size=14/>"Stopp"</button>
                    </div>
                </Show>
            </Show>
        </div>
    }
}

#[component]
fn ShowControls(conn: RoomConn, show: ShowInfo) -> impl IntoView {
    let event_id = show.event_id;
    let live = show.status == EventStatus::Live;
    let in_break = move || conn.state.with(|s| matches!(s, PlaybackState::Intermission { .. }));
    let in_item = move || live && conn.state.with(|s| s.media().is_some());
    view! {
        <div class="show-controls">
            {(!live).then(|| view! {
                <ArmedButton label="▶ Vorstellung jetzt starten" on_fire=Callback::new(move |_| conn.control(ControlAction::StartEvent, |a| a.event_id = Some(event_id)))/>
            })}
            <Show when=in_item>
                <ArmedButton label="⏭ Film überspringen" on_fire=Callback::new(move |_| conn.control(ControlAction::SkipItem, |_| {}))/>
            </Show>
            <Show when=in_break>
                <button class="btn btn-sm btn-ghost" on:click=move |_| conn.control(ControlAction::ExtendIntermission, |a| a.delta_ms = Some(60_000))>"+1 Min."</button>
                <button class="btn btn-sm btn-ghost" on:click=move |_| conn.control(ControlAction::ExtendIntermission, |a| a.delta_ms = Some(-60_000))>"−1 Min."</button>
                <button class="btn btn-sm" on:click=move |_| conn.control(ControlAction::EndIntermission, |_| {})><Icon name="play" size=14/>"Pause beenden"</button>
            </Show>
            <span class="grow"></span>
            {if live {
                view! { <ArmedButton label="⏹ Vorstellung beenden" danger=true on_fire=Callback::new(move |_| conn.control(ControlAction::EndEvent, |_| {}))/> }.into_any()
            } else {
                view! { <ArmedButton label="✕ Absagen" armed_label="Sicher? Vorstellung absagen" danger=true on_fire=Callback::new(move |_| conn.control(ControlAction::EndEvent, |_| {}))/> }.into_any()
            }}
        </div>
    }
}

#[component]
fn Rundown(show: ShowInfo) -> impl IntoView {
    let current = show.current_index;
    let count = show.items.len();
    view! {
        <ol class="rundown">
            {show.items.into_iter().enumerate().map(|(i, it)| {
                let played = current.is_some_and(|c| i < c);
                let active = current == Some(i);
                view! {
                    <li class="rundown-item" class:active=active class:played=played>
                        <span class="rundown-mark">{if played { "✓".to_string() } else if active { "▶".to_string() } else { (i + 1).to_string() }}</span>
                        <span class="grow">
                            {it.title.clone()}
                            {(!it.ready).then(|| view! { " " <span class="tag-chip danger">"nicht bereit"</span> })}
                        </span>
                        <span class="muted small">
                            {it.duration_ms.map(|d| fmt_timecode(d as f64)).unwrap_or_else(|| "–:––".into())}
                            {(i + 1 < count && it.intermission_after_ms > 0).then(|| format!(" · {} Min. Pause danach", it.intermission_after_ms / 60_000))}
                        </span>
                    </li>
                }
            }).collect_view()}
        </ol>
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
struct FormItem {
    key: u64,
    media_id: Uuid,
    title: String,
    duration_ms: Option<i64>,
    ready: bool,
    break_min: String,
}

fn fmt_dur(ms: i64) -> String {
    let m = (ms as f64 / 60_000.0).round() as i64;
    if m >= 60 {
        format!("{} h {:02} min", m / 60, m % 60)
    } else {
        format!("{m} min")
    }
}

fn runtime_label(items: &[(Option<i64>, i64)]) -> String {
    let mut total = 0;
    let mut unknown = 0;
    for (i, (d, brk)) in items.iter().enumerate() {
        match d {
            Some(d) => total += d,
            None => unknown += 1,
        }
        if i + 1 < items.len() {
            total += brk;
        }
    }
    if items.is_empty() {
        String::new()
    } else if unknown > 0 {
        format!("{} + {unknown}× unbekannte Länge", fmt_dur(total))
    } else {
        fmt_dur(total)
    }
}

#[component]
fn EventScheduler(
    conn: RoomConn,
    version: RwSignal<u32>,
    prefill: RwSignal<Option<(u64, String, Option<Uuid>)>>,
) -> impl IntoView {
    let toasts = use_toasts();
    let include_past = RwSignal::new(false);
    let events = LocalResource::new(move || {
        version.track();
        let past = include_past.get();
        async move { watch_events(conn.room_id, past).await.unwrap_or_default() }
    });
    let media = LocalResource::new(|| async { watch_library().await.map(|l| l.media).unwrap_or_default() });
    // None = closed, Some(None) = new, Some(Some(id)) = editing.
    let editing = RwSignal::new(None::<Option<Uuid>>);
    let title = RwSignal::new(String::new());
    let starts = RwSignal::new(String::new());
    let items = RwSignal::new(Vec::<FormItem>::new());
    let pick = RwSignal::new(String::new());
    let warnings = RwSignal::new(Vec::<String>::new());
    let busy = RwSignal::new(false);
    let promoting = RwSignal::new(None::<Uuid>);
    let next_key = StoredValue::new(1u64);

    let open_new = move || {
        editing.set(Some(None));
        title.set(String::new());
        let start = chrono::Utc::now() + chrono::Duration::hours(1);
        starts.set(datetime_local_value(start));
        items.set(Vec::new());
        warnings.set(Vec::new());
    };
    Effect::new(move |_| {
        if let Some((_, t, proposal)) = prefill.get() {
            open_new();
            title.set(t);
            promoting.set(proposal);
        }
    });
    let open_edit = move |ev: EventInfo| {
        editing.set(Some(Some(ev.id)));
        title.set(ev.title.clone());
        starts.set(chrono::DateTime::from_timestamp_millis(ev.starts_at_ms).map(datetime_local_value).unwrap_or_default());
        items.set(
            ev.items
                .iter()
                .map(|i| {
                    let k = next_key.get_value();
                    next_key.set_value(k + 1);
                    FormItem {
                        key: k,
                        media_id: i.media_id,
                        title: i.title.clone(),
                        duration_ms: i.duration_ms,
                        ready: i.ready,
                        break_min: (i.intermission_after_ms / 60_000).to_string(),
                    }
                })
                .collect(),
        );
        warnings.set(Vec::new());
    };
    let add_item = move |_| {
        let Ok(id) = Uuid::parse_str(&pick.get_untracked()) else { return };
        let Some(m): Option<MediaInfo> = local(media).unwrap_or_default().into_iter().find(|m| m.id == id) else { return };
        let k = next_key.get_value();
        next_key.set_value(k + 1);
        items.update(|v| {
            v.push(FormItem {
                key: k,
                media_id: m.id,
                title: m.title.clone(),
                duration_ms: m.duration_ms,
                ready: m.status == MediaStatus::Ready,
                break_min: "15".into(),
            })
        });
    };
    let save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Some(mode) = editing.get_untracked() else { return };
        let Some(start) = parse_datetime_local(&starts.get_untracked()) else {
            toasts.error("Bitte gib einen gültigen Beginn an.");
            return;
        };
        let input = EventInput {
            title: title.get_untracked(),
            starts_at_ms: start.timestamp_millis(),
            items: items
                .get_untracked()
                .iter()
                .map(|i| EventItemInput {
                    media_id: i.media_id,
                    intermission_after_ms: i.break_min.trim().parse::<i64>().unwrap_or(0).max(0) * 60_000,
                })
                .collect(),
        };
        busy.set(true);
        leptos::task::spawn_local(async move {
            let res = match mode {
                Some(id) => update_watch_event(id, input).await,
                None => create_watch_event(conn.room_id, input).await,
            };
            busy.set(false);
            match res {
                Ok(saved) => {
                    if let Some(p) = promoting.get_untracked() {
                        let _ = set_watch_proposal_status(p, ProposalStatus::Scheduled).await;
                        promoting.set(None);
                    }
                    if saved.warnings.is_empty() {
                        editing.set(None);
                        toasts.success("Vorstellung gespeichert.");
                    } else {
                        editing.set(Some(Some(saved.event.id)));
                    }
                    warnings.set(saved.warnings);
                    version.update(|v| *v += 1);
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let act = move |fut: std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::error::AppError>>>>| {
        leptos::task::spawn_local(async move {
            match fut.await {
                Ok(()) => version.update(|v| *v += 1),
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    let live_id = move || conn.show.with(|s| s.as_ref().filter(|s| s.status == EventStatus::Live).map(|s| s.event_id));
    let item_keys = move || items.get();
    let item_key = |i: &FormItem| i.key;

    view! {
        <div class="scheduler">
            <div class="row gap-sm">
                <button class="btn btn-sm btn-primary" on:click=move |_| open_new()><Icon name="plus" size=14/>"Neue Vorstellung"</button>
                <label class="check small"><input type="checkbox" bind:checked=include_past/><span>"Vergangene zeigen"</span></label>
            </div>
            <Show when=move || editing.get().is_some()>
                <form class="form card event-form" on:submit=save>
                    <div class="form-row">
                        <label class="field grow"><span>"Titel"</span><input type="text" required maxlength="120" bind:value=title/></label>
                        <label class="field"><span>"Beginn"</span><input type="datetime-local" required bind:value=starts/></label>
                    </div>
                    <ol class="lineup">
                        <For each=item_keys key=item_key children=move |it: FormItem| {
                            let k = it.key;
                            let brk = RwSignal::new(it.break_min.clone());
                            Effect::new(move |_| {
                                let v = brk.get();
                                items.update(|list| if let Some(x) = list.iter_mut().find(|x| x.key == k) { x.break_min = v; });
                            });
                            let move_by = move |d: i64| items.update(|list| {
                                if let Some(pos) = list.iter().position(|x| x.key == k) {
                                    let to = pos as i64 + d;
                                    if to >= 0 && (to as usize) < list.len() { list.swap(pos, to as usize); }
                                }
                            });
                            view! {
                                <li class="lineup-item">
                                    <span class="grow">
                                        {it.title.clone()}
                                        <span class="muted small">{it.duration_ms.map(|d| format!(" · {}", fmt_timecode(d as f64))).unwrap_or_else(|| " · Länge unbekannt".into())}</span>
                                        {(!it.ready).then(|| view! { " " <span class="tag-chip danger">"nicht bereit"</span> })}
                                    </span>
                                    <label class="small row gap-sm">"Pause danach"<input class="mini" type="number" min="0" max="240" bind:value=brk/>"Min."</label>
                                    <button type="button" class="icon-btn" title="Nach oben" on:click=move |_| move_by(-1)><Icon name="arrow-up" size=14/></button>
                                    <button type="button" class="icon-btn" title="Nach unten" on:click=move |_| move_by(1)><Icon name="arrow-down" size=14/></button>
                                    <button type="button" class="icon-btn" title="Entfernen" on:click=move |_| items.update(|l| l.retain(|x| x.key != k))><Icon name="x" size=14/></button>
                                </li>
                            }
                        }/>
                    </ol>
                    <div class="row gap-sm">
                        <select class="grow" prop:value=move || pick.get() on:change=move |ev| pick.set(event_target_value(&ev))>
                            <option value="">"Film aus der Mediathek hinzufügen …"</option>
                            {move || local(media).unwrap_or_default().into_iter().map(|m| view! {
                                <option value=m.id.to_string()>{format!("{}{}", m.title, if m.status == MediaStatus::Ready { "" } else { " (noch nicht bereit)" })}</option>
                            }).collect_view()}
                        </select>
                        <button type="button" class="btn btn-sm" disabled=move || pick.get().is_empty() on:click=add_item><Icon name="plus" size=14/>"Hinzufügen"</button>
                    </div>
                    <p class="muted small">{move || {
                        let list = items.get();
                        let rt: Vec<(Option<i64>, i64)> = list.iter().map(|i| (i.duration_ms, i.break_min.trim().parse::<i64>().unwrap_or(0) * 60_000)).collect();
                        let label = runtime_label(&rt);
                        if label.is_empty() { "Noch keine Filme im Line-up.".to_string() } else { format!("Gesamtdauer: {label}") }
                    }}</p>
                    {move || {
                        let w = warnings.get();
                        (!w.is_empty()).then(|| view! {
                            <div class="alert small"><ul>{w.into_iter().map(|x| view! { <li>{x}</li> }).collect_view()}</ul></div>
                        })
                    }}
                    <div class="form-actions">
                        <button type="button" class="btn" on:click=move |_| { editing.set(None); promoting.set(None); }>"Schließen"</button>
                        <button type="submit" class="btn btn-primary" disabled=move || busy.get() || items.with(|i| i.is_empty())>"Speichern"</button>
                    </div>
                </form>
            </Show>
            <ul class="plain-list event-list">
                {move || {
                    let list = local(events).unwrap_or_default();
                    if list.is_empty() {
                        return view! { <li class="muted small">"Keine Vorstellungen geplant."</li> }.into_any();
                    }
                    let live_now = live_id();
                    list.into_iter().map(|ev| {
                        let id = ev.id;
                        let status = ev.status;
                        let rt: Vec<(Option<i64>, i64)> = ev.items.iter().map(|i| (i.duration_ms, i.intermission_after_ms)).collect();
                        let edit_ev = ev.clone();
                        view! {
                            <li class="event-row">
                                <span class=format!("tag-chip status-{}", status.as_str())>{status.label()}</span>
                                <div class="grow">
                                    <strong>{ev.title.clone()}</strong>
                                    <div class="muted small">
                                        {chrono::DateTime::from_timestamp_millis(ev.starts_at_ms).map(local_datetime).unwrap_or_default()}
                                        {format!(" · {} Filme · {}", ev.items.len(), runtime_label(&rt))}
                                    </div>
                                </div>
                                {(status == EventStatus::Scheduled).then(|| view! {
                                    {live_now.is_none().then(|| view! {
                                        <ArmedButton label="▶ Jetzt starten" on_fire=Callback::new(move |_| conn.control(ControlAction::StartEvent, |a| a.event_id = Some(id)))/>
                                    })}
                                    <button class="btn btn-sm btn-ghost" on:click=move |_| open_edit(edit_ev.clone())><Icon name="edit" size=14/></button>
                                    <button class="btn btn-sm btn-ghost" title="Absagen" on:click=move |_| if confirm("Vorstellung absagen?") { act(Box::pin(cancel_watch_event(id))) }>"Absagen"</button>
                                })}
                                {(status != EventStatus::Live).then(|| view! {
                                    <button class="icon-btn" title="Löschen" on:click=move |_| if confirm("Vorstellung löschen?") { act(Box::pin(delete_watch_event(id))) }><Icon name="trash" size=14/></button>
                                })}
                            </li>
                        }
                    }).collect_view().into_any()
                }}
            </ul>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_labels() {
        let min = 60_000;
        assert_eq!(runtime_label(&[(Some(90 * min), 15 * min), (Some(100 * min), 30 * min)]), "3 h 25 min");
        assert_eq!(runtime_label(&[(None, 0), (Some(10 * min), 0)]), "10 min + 1× unbekannte Länge");
        assert_eq!(runtime_label(&[]), "");
    }
}
