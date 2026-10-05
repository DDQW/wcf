//! Integration tests against a real PostgreSQL server. Every test creates
//! its own throwaway database (`wcf_test_…`), runs all migrations and drops
//! it again. Tests are skipped when neither `TEST_DATABASE_URL` nor
//! `DATABASE_URL` (also read from the repository's `.env`) is set; the
//! account needs the right to create databases.

use backend::auth::{self, RegisterInput};
use backend::{AppState, Config};
use shared::admin::InviteInput;
use shared::call::{CallRoomInput, RoomVisibility};
use shared::forum::{CategoryAccess, CategoryInput, NewTopic};
use shared::watch::{ModerationKind, PlaylistMode, PlaylistSource, RoomRole, WatchRoomInput, WatchVisibility};
use shared::{AuthUser, Role};
use sqlx::PgPool;
use uuid::Uuid;

struct TestDb {
    state: AppState,
    admin_url: String,
    name: String,
}

fn split_url(url: &str) -> Option<(String, String)> {
    let (base, query) = match url.split_once('?') {
        Some((b, q)) => (b, format!("?{q}")),
        None => (url, String::new()),
    };
    let (prefix, _db) = base.rsplit_once('/')?;
    Some((prefix.to_string(), query))
}

async fn setup() -> Option<TestDb> {
    let _ = dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env"));
    let url = std::env::var("TEST_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL")).ok()?;
    let (prefix, query) = split_url(&url)?;
    let admin_url = format!("{prefix}/postgres{query}");
    let admin = PgPool::connect(&admin_url).await.ok()?;
    let name = format!("wcf_test_{}", Uuid::now_v7().simple());
    sqlx::query(&format!("CREATE DATABASE {name}")).execute(&admin).await.ok()?;
    admin.close().await;
    let db_url = format!("{prefix}/{name}{query}");
    let pool = PgPool::connect(&db_url).await.expect("connect test db");
    backend::migrate(&pool).await.expect("migrations");
    let config = Config {
        database_url: db_url,
        public_url: "http://localhost:3000".into(),
        data_dir: std::env::temp_dir().join(&name),
        cookie_secure: false,
        trust_proxy: false,
        smtp: None,
        livekit: None,
        ffmpeg: None,
        watch_import_dir: None,
        watch_max_media_bytes: None,
    };
    let state = AppState::new(config, pool).await.expect("state");
    Some(TestDb { state, admin_url, name })
}

impl TestDb {
    async fn teardown(self) {
        let data_dir = self.state.config.data_dir.clone();
        self.state.db.close().await;
        if let Ok(admin) = PgPool::connect(&self.admin_url).await {
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name))
                .execute(&admin)
                .await;
        }
        let _ = std::fs::remove_dir_all(data_dir);
    }

    async fn register(&self, name: &str, invite: &str) -> backend::Result<AuthUser> {
        auth::register(
            &self.state,
            &format!("10.0.0.{}", name.len()),
            RegisterInput {
                username: name.into(),
                email: format!("{name}@example.test"),
                password: "geheim-und-lang-1".into(),
                display_name: String::new(),
                invite_code: invite.into(),
            },
        )
        .await
    }
}

macro_rules! db_or_skip {
    () => {
        match setup().await {
            Some(db) => db,
            None => {
                eprintln!("skipping: no database configured");
                return;
            }
        }
    };
}

#[tokio::test]
async fn registration_roles_and_invites() {
    let t = db_or_skip!();
    let admin = t.register("anna", "").await.expect("first user");
    assert_eq!(admin.role, Role::Admin, "the first account becomes admin");
    let bob = t.register("bob", "").await.expect("second user");
    assert_eq!(bob.role, Role::Member);
    assert!(t.register("Bob", "").await.is_err(), "usernames are case-insensitive unique");

    // Invite-only mode.
    let mut settings = t.state.settings();
    settings.invite_only = true;
    backend::settings::save(&t.state, &admin, settings).await.expect("save settings");
    assert!(t.register("carl", "").await.is_err(), "a code is required");
    assert!(backend::invites::create(&t.state, &bob, InviteInput { count: 1, max_uses: 1, expires_days: None, note: String::new() })
        .await
        .is_err(), "members may not invite unless allowed");
    let codes = backend::invites::create(&t.state, &admin, InviteInput { count: 1, max_uses: 1, expires_days: Some(7), note: "test".into() })
        .await
        .expect("create invite");
    let code = codes[0].code.clone();
    // A failed registration must not burn the code.
    assert!(t.register("bob", &code).await.is_err());
    let carl = t.register("carl", &code.to_lowercase().replace('-', " ")).await.expect("invite accepted");
    assert_eq!(carl.role, Role::Member);
    assert!(t.register("dora", &code).await.is_err(), "single-use code is spent");
    let list = backend::invites::list(&t.state, &admin).await.unwrap();
    assert_eq!(list[0].uses, 1);
    assert_eq!(list[0].used_by[0].username, "carl");
    t.teardown().await;
}

#[tokio::test]
async fn login_checks_password() {
    let t = db_or_skip!();
    t.register("anna", "").await.unwrap();
    assert!(auth::login(&t.state, "10.1.1.1", "anna", "falsch-falsch").await.is_err());
    let user = auth::login(&t.state, "10.1.1.1", "ANNA@example.test", "geheim-und-lang-1").await.expect("login by e-mail");
    assert_eq!(user.username, "anna");
    t.teardown().await;
}

#[tokio::test]
async fn forum_read_access() {
    let t = db_or_skip!();
    let admin = t.register("anna", "").await.unwrap();
    let member = t.register("bob", "").await.unwrap();
    let category = |name: &str, read_access| CategoryInput {
        name: name.into(),
        description: String::new(),
        color: "#4f46e5".into(),
        parent_id: None,
        read_access,
        post_role: Role::Member,
        position: 0,
    };
    let public = backend::forum::create_category(&t.state, &admin, category("Allgemein", CategoryAccess::Public)).await.unwrap();
    let staff = backend::forum::create_category(&t.state, &admin, category("Team", CategoryAccess::Staff)).await.unwrap();
    let topic = |category_id| NewTopic { category_id, title: "Hallo Welt".into(), body_md: "Erster Beitrag".into(), tags: vec![] };
    let open = backend::forum::create_topic(&t.state, &member, topic(public)).await.expect("members post publicly");
    assert!(backend::forum::create_topic(&t.state, &member, topic(staff)).await.is_err(), "members cannot post in the team area");
    let internal = backend::forum::create_topic(&t.state, &admin, topic(staff)).await.unwrap();
    let db = &t.state.db;
    assert!(backend::forum::can_read_topic(db, None, open).await.unwrap(), "guests read public topics");
    assert!(!backend::forum::can_read_topic(db, Some(&member), internal).await.unwrap());
    assert!(backend::forum::can_read_topic(db, Some(&admin), internal).await.unwrap());
    let reply = backend::forum::create_post(&t.state, &admin, open, None, "Willkommen @bob!").await.expect("reply");
    assert!(reply.body_html.contains("bob"));
    t.teardown().await;
}

#[tokio::test]
async fn watch_room_access_moderation_and_playlist() {
    let t = db_or_skip!();
    let owner = t.register("anna", "").await.unwrap();
    let guest = t.register("bob", "").await.unwrap();
    let room = backend::watch::create_room(
        &t.state,
        &owner,
        WatchRoomInput {
            name: "Filmabend".into(),
            description: String::new(),
            visibility: WatchVisibility::Private,
            playlist_mode: PlaylistMode::Members,
            voteskip_percent: 50,
        },
    )
    .await
    .expect("create room");
    assert_eq!(room.slug, "filmabend");
    let db = &t.state.db;
    assert!(backend::watch::accessible(db, &guest, room.id).await.is_err(), "private room is hidden");
    backend::watch::set_role(&t.state, &owner, room.id, guest.id, Some(RoomRole::Member)).await.unwrap();
    let (_, role) = backend::watch::accessible(db, &guest, room.id).await.expect("invited member");
    assert_eq!(role, Some(RoomRole::Member));

    // Members may fill the playlist in "members" mode.
    backend::watch_live::add_to_playlist(
        &t.state,
        &guest,
        room.id,
        PlaylistSource::Url { url: "https://example.org/videos/Mein%20Urlaub.mp4".into() },
        false,
    )
    .await
    .expect("add url");
    let rejected = backend::watch_live::add_to_playlist(
        &t.state,
        &guest,
        room.id,
        PlaylistSource::Url { url: "https://example.org/page.html".into() },
        false,
    )
    .await;
    assert!(rejected.is_err(), "non-video links are rejected");
    let items = backend::watch::playlist(db, room.id).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "Mein Urlaub");

    // Moderation: a ban locks the user out, an unban lets them back in.
    assert!(backend::watch::moderate(&t.state, &guest, room.id, owner.id, ModerationKind::Ban, None, "").await.is_err());
    backend::watch::moderate(&t.state, &owner, room.id, guest.id, ModerationKind::Ban, None, "Spam").await.unwrap();
    assert!(backend::watch::accessible(db, &guest, room.id).await.is_err());
    backend::watch::moderate(&t.state, &owner, room.id, guest.id, ModerationKind::Unban, None, "").await.unwrap();
    assert!(backend::watch::accessible(db, &guest, room.id).await.is_ok());
    t.teardown().await;
}

#[tokio::test]
async fn private_call_rooms_guard_their_chat() {
    let t = db_or_skip!();
    let owner = t.register("anna", "").await.unwrap();
    let outsider = t.register("bob", "").await.unwrap();
    let room = backend::calls::create_room(
        &t.state,
        &owner,
        CallRoomInput { name: "Team".into(), description: String::new(), visibility: RoomVisibility::Private, persistent: true },
    )
    .await
    .unwrap();
    let detail = backend::calls::room_detail(&t.state, &owner, room.id).await.unwrap();
    let channel = detail.chat_channel_id.expect("room chat");
    let db = &t.state.db;
    assert!(backend::calls::room_detail(&t.state, &outsider, room.id).await.is_err());
    assert!(!backend::chat::can_subscribe(db, Some(&outsider), channel).await, "outsiders cannot read the room chat");
    backend::calls::set_member(&t.state, &owner, room.id, outsider.id, Some(false)).await.unwrap();
    assert!(backend::chat::can_subscribe(db, Some(&outsider), channel).await, "invited members can");
    // No LiveKit configured: joining explains why instead of handing out a token.
    assert!(backend::calls::join_room(&t.state, &owner, room.id).await.is_err());
    t.teardown().await;
}
