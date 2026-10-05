use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;

/// Foundation-only WebSocket route: proves the transport works
/// end-to-end (see plan verification step 11). Not wired into any
/// feature yet — chat is the first feature to use it for real, via a
/// `tokio::sync::broadcast` room registry.
pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    while let Some(Ok(msg)) = socket.recv().await {
        if let Message::Text(text) = msg {
            if socket.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    }
}
