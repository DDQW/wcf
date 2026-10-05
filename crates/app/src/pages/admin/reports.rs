use leptos::prelude::*;
use shared::moderation::{ReportStatus, ReportView};

use crate::api::moderation::{list_reports, resolve_report};
use crate::components::{Avatar, EmptyState, ErrorBox, Icon, Pager, Spinner, TimeAgo};
use crate::state::use_toasts;

#[component]
pub fn AdminReports() -> impl IntoView {
    let status = RwSignal::new(ReportStatus::Open);
    let page = RwSignal::new(0i64);
    let version = RwSignal::new(0u32);
    let data = Resource::new(move || (status.get(), page.get(), version.get()), |(s, p, _)| list_reports(s, p));
    let total = Signal::derive(move || data.get().and_then(|r| r.ok()).map(|p| p.total).unwrap_or(0));
    let refresh = Callback::new(move |_| version.update(|v| *v += 1));
    let tab = move |s: ReportStatus| view! {
        <button class="tab" class:active=move || status.get() == s on:click=move |_| { status.set(s); page.set(0); }>{s.label()}</button>
    };
    view! {
        <h1>"Meldungen"</h1>
        <div class="tabs">
            {tab(ReportStatus::Open)}
            {tab(ReportStatus::Resolved)}
            {tab(ReportStatus::Dismissed)}
        </div>
        <Transition fallback=|| view! { <Spinner/> }>
            {move || Suspend::new(async move {
                match data.await {
                    Err(e) => view! { <ErrorBox error=e/> }.into_any(),
                    Ok(p) if p.items.is_empty() => view! { <EmptyState icon="check-circle" title="Keine Meldungen."/> }.into_any(),
                    Ok(p) => view! {
                        <div class="stack">
                            {p.items.into_iter().map(|r| view! { <ReportCard r=r on_change=refresh/> }).collect_view()}
                        </div>
                    }.into_any(),
                }
            })}
        </Transition>
        <Pager page=page total=total per_page=20/>
    }
}

#[component]
fn ReportCard(r: ReportView, on_change: Callback<()>) -> impl IntoView {
    let toasts = use_toasts();
    let note = RwSignal::new(String::new());
    let delete_content = RwSignal::new(false);
    let id = r.id;
    let open = r.status == ReportStatus::Open;
    let act = move |status: ReportStatus| {
        leptos::task::spawn_local(async move {
            match resolve_report(id, status, note.get_untracked(), delete_content.get_untracked()).await {
                Ok(()) => {
                    toasts.success("Meldung bearbeitet.");
                    on_change.run(());
                }
                Err(e) => toasts.error(e.to_string()),
            }
        });
    };
    view! {
        <article class="card report">
            <header class="row gap wrap">
                <span class="badge">{r.subject_type.label()}</span>
                <strong>{r.reason.label()}</strong>
                <span class="muted small">"gemeldet von " <a href=r.reporter.profile_url()>{r.reporter.display_name.clone()}</a> " · " <TimeAgo ts=r.created_at/></span>
            </header>
            <blockquote class="report-preview">
                {r.content_author.clone().map(|a| view! { <div class="row gap-sm small"><Avatar user=a.clone() size=20/><a href=a.profile_url()>{a.display_name.clone()}</a></div> })}
                <p>{if r.preview.is_empty() { "(Inhalt nicht mehr vorhanden)".to_string() } else { r.preview.clone() }}</p>
                {r.link.clone().map(|l| view! { <a class="small" href=l><Icon name="external-link" size=14/>"Im Kontext ansehen"</a> })}
            </blockquote>
            {(!r.details.is_empty()).then(|| view! { <p class="small"><strong>"Details: "</strong>{r.details.clone()}</p> })}
            {if open {
                view! {
                    <div class="stack-sm">
                        <input type="text" placeholder="Notiz (optional, sieht der Meldende)" bind:value=note/>
                        <div class="row gap wrap">
                            <label class="check"><input type="checkbox" bind:checked=delete_content/>"Inhalt löschen"</label>
                            <span class="grow"></span>
                            <button class="btn" on:click=move |_| act(ReportStatus::Dismissed)>"Verwerfen"</button>
                            <button class="btn btn-primary" on:click=move |_| act(ReportStatus::Resolved)><Icon name="check" size=16/>"Erledigt"</button>
                        </div>
                    </div>
                }.into_any()
            } else {
                view! {
                    <p class="muted small">
                        {r.status.label()}
                        {r.resolved_by.clone().map(|u| format!(" von {}", u.display_name))}
                        {r.resolved_at.map(|t| view! { " · " <TimeAgo ts=t/> })}
                        {r.resolution_note.clone().map(|n| format!(" – {n}"))}
                    </p>
                }.into_any()
            }}
        </article>
    }
}
