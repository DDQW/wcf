//! Browser uploads to `POST /api/files` via XMLHttpRequest (for progress).

#[cfg(feature = "hydrate")]
use shared::files::UploadResult;
use shared::files::{FilePurpose, Visibility};
use uuid::Uuid;

#[derive(Clone, Debug, Default)]
pub struct UploadOpts {
    pub folder_id: Option<Uuid>,
    pub purpose: FilePurpose,
    pub visibility: Option<Visibility>,
    pub context_id: Option<Uuid>,
}

/// Uploads one file. `on_progress` receives values from 0.0 to 1.0.
#[cfg(feature = "hydrate")]
pub async fn upload_file(
    file: web_sys::File,
    opts: UploadOpts,
    on_progress: impl Fn(f64) + 'static,
) -> Result<UploadResult, String> {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    let xhr = web_sys::XmlHttpRequest::new().map_err(|_| "Upload nicht möglich")?;
    xhr.open("POST", "/api/files").map_err(|_| "Upload nicht möglich")?;
    let form = web_sys::FormData::new().map_err(|_| "Upload nicht möglich")?;
    if let Some(f) = opts.folder_id {
        let _ = form.append_with_str("folder_id", &f.to_string());
    }
    let _ = form.append_with_str("purpose", opts.purpose.as_str());
    if let Some(v) = opts.visibility {
        let _ = form.append_with_str("visibility", v.as_str());
    }
    if let Some(c) = opts.context_id {
        let _ = form.append_with_str("context_id", &c.to_string());
    }
    form.append_with_blob_and_filename("file", &file, &file.name())
        .map_err(|_| "Datei konnte nicht gelesen werden")?;

    let progress = Closure::<dyn FnMut(web_sys::ProgressEvent)>::new(move |e: web_sys::ProgressEvent| {
        if e.length_computable() && e.total() > 0.0 {
            on_progress(e.loaded() / e.total());
        }
    });
    if let Ok(upload) = xhr.upload() {
        upload.set_onprogress(Some(progress.as_ref().unchecked_ref()));
    }

    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let onload = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
        xhr.set_onload(Some(onload.unchecked_ref()));
        let reject2 = reject.clone();
        let onerror = Closure::once_into_js(move || {
            let _ = reject.call0(&JsValue::NULL);
        });
        xhr.set_onerror(Some(onerror.unchecked_ref()));
        let onabort = Closure::once_into_js(move || {
            let _ = reject2.call0(&JsValue::NULL);
        });
        xhr.set_onabort(Some(onabort.unchecked_ref()));
    });
    xhr.send_with_opt_form_data(Some(&form))
        .map_err(|_| "Upload nicht möglich")?;
    let outcome = wasm_bindgen_futures::JsFuture::from(promise).await;
    drop(progress);
    if outcome.is_err() {
        return Err("Netzwerkfehler beim Hochladen.".into());
    }
    let status = xhr.status().unwrap_or(0);
    let text = xhr.response_text().ok().flatten().unwrap_or_default();
    if status == 200 {
        serde_json::from_str::<UploadResult>(&text).map_err(|_| "Unerwartete Antwort vom Server.".into())
    } else {
        let msg = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| format!("Upload fehlgeschlagen ({status})."));
        Err(msg)
    }
}

/// Files from an `<input type=file>` change event.
#[cfg(feature = "hydrate")]
pub fn files_from_input(ev: &leptos::ev::Event) -> Vec<web_sys::File> {
    use wasm_bindgen::JsCast;
    let Some(input) = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(list) = input.files() {
        for i in 0..list.length() {
            if let Some(f) = list.get(i) {
                out.push(f);
            }
        }
    }
    input.set_value("");
    out
}

/// Files from a drop event.
#[cfg(feature = "hydrate")]
pub fn files_from_drop(ev: &web_sys::DragEvent) -> Vec<web_sys::File> {
    let mut out = Vec::new();
    if let Some(list) = ev.data_transfer().and_then(|dt| dt.files()) {
        for i in 0..list.length() {
            if let Some(f) = list.get(i) {
                out.push(f);
            }
        }
    }
    out
}
