use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};

pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
	ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
	let _ = socket.send(Message::text("connected")).await;
	while let Some(Ok(msg)) = socket.next().await {
		match msg {
			Message::Text(_t) => {
				let _ = socket.send(Message::text("pong")).await;
			}
			Message::Close(_) => break,
			_ => {}
		}
	}
}


