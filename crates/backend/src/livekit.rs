//! LiveKit integration: access tokens (HS256 JWTs) for browsers and the
//! server-side RoomService API (Twirp/JSON) for moderation.

use std::sync::OnceLock;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use crate::config::LiveKitConfig;
use crate::error::{Error, Result};

/// Permissions inside one LiveKit room.
#[derive(Debug, Clone)]
pub struct Grant {
    pub room: String,
    pub admin: bool,
    pub can_publish: bool,
}

fn sign(cfg: &LiveKitConfig, claims: &Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
    let input = format!("{header}.{payload}");
    let mut mac = Hmac::<Sha256>::new_from_slice(cfg.api_secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(input.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{input}.{sig}")
}

/// Access token for a participant. `identity` must be unique per room.
pub fn participant_token(
    cfg: &LiveKitConfig,
    identity: &str,
    name: &str,
    metadata: &Value,
    grant: &Grant,
    ttl: Duration,
) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": cfg.api_key,
        "sub": identity,
        "name": name,
        "metadata": metadata.to_string(),
        "nbf": now - 10,
        "exp": now + ttl.as_secs() as i64,
        "video": {
            "room": grant.room,
            "roomJoin": true,
            "roomAdmin": grant.admin,
            "canPublish": grant.can_publish,
            "canSubscribe": true,
            "canPublishData": true,
            "canUpdateOwnMetadata": false,
        },
    });
    sign(cfg, &claims)
}

fn admin_token(cfg: &LiveKitConfig, room: &str) -> String {
    let now = chrono::Utc::now().timestamp();
    sign(
        cfg,
        &json!({
            "iss": cfg.api_key,
            "sub": "wcf-server",
            "nbf": now - 10,
            "exp": now + 120,
            "video": { "room": room, "roomAdmin": true, "roomList": true },
        }),
    )
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client")
    })
}

async fn room_service(cfg: &LiveKitConfig, method: &str, room: &str, body: Value) -> Result<Value> {
    let url = format!("{}/twirp/livekit.RoomService/{method}", cfg.api_url);
    let res = client()
        .post(&url)
        .bearer_auth(admin_token(cfg, room))
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::invalid(format!("Medienserver nicht erreichbar ({e}).")))?;
    let status = res.status();
    let value: Value = res.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = value.get("msg").and_then(|m| m.as_str()).unwrap_or("unbekannter Fehler");
        return Err(Error::invalid(format!("Medienserver: {msg}")));
    }
    Ok(value)
}

/// Mutes all microphone tracks a participant publishes.
pub async fn mute_microphone(cfg: &LiveKitConfig, room: &str, identity: &str) -> Result<usize> {
    let list = room_service(cfg, "ListParticipants", room, json!({ "room": room })).await?;
    let participant = list
        .get("participants")
        .and_then(|p| p.as_array())
        .and_then(|ps| ps.iter().find(|p| p.get("identity").and_then(|i| i.as_str()) == Some(identity)))
        .cloned();
    let Some(p) = participant else { return Ok(0) };
    let mut muted = 0;
    for track in p.get("tracks").and_then(|t| t.as_array()).cloned().unwrap_or_default() {
        // Enum values may come as names or numbers; AUDIO = 0 may be omitted.
        let is_audio = match track.get("type") {
            None => true,
            Some(Value::String(s)) => s == "AUDIO",
            Some(Value::Number(n)) => n.as_i64() == Some(0),
            _ => false,
        };
        let is_mic = match track.get("source") {
            Some(Value::String(s)) => s == "MICROPHONE",
            Some(Value::Number(n)) => n.as_i64() == Some(2),
            _ => is_audio,
        };
        let Some(sid) = track.get("sid").and_then(|s| s.as_str()) else { continue };
        if is_audio && is_mic {
            room_service(
                cfg,
                "MutePublishedTrack",
                room,
                json!({ "room": room, "identity": identity, "track_sid": sid, "muted": true }),
            )
            .await?;
            muted += 1;
        }
    }
    Ok(muted)
}

pub async fn remove_participant(cfg: &LiveKitConfig, room: &str, identity: &str) -> Result<()> {
    room_service(cfg, "RemoveParticipant", room, json!({ "room": room, "identity": identity })).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_has_three_parts_and_valid_signature() {
        let cfg = LiveKitConfig {
            url: "ws://localhost:7880".into(),
            api_url: "http://localhost:7880".into(),
            api_key: "devkey".into(),
            api_secret: "secret".into(),
        };
        let t = participant_token(
            &cfg,
            "user-1",
            "Alice",
            &json!({}),
            &Grant { room: "room-1".into(), admin: false, can_publish: true },
            Duration::from_secs(60),
        );
        let parts: Vec<&str> = t.split('.').collect();
        assert_eq!(parts.len(), 3);
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(format!("{}.{}", parts[0], parts[1]).as_bytes());
        let sig = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        mac.verify_slice(&sig).unwrap();
        let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["sub"], "user-1");
        assert_eq!(claims["video"]["room"], "room-1");
        assert_eq!(claims["video"]["roomJoin"], true);
    }
}
