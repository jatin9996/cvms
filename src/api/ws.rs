use axum::extract::{State};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::rpc_config::RpcAccountInfoConfig;
use solana_sdk::pubkey::Pubkey;

use super::AppState;

pub async fn ws_handler(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(state, socket))
}

#[derive(Deserialize)]
struct SubscribeMsg { subscribe: String }

async fn handle_socket(state: AppState, mut socket: WebSocket) {
	let _ = socket.send(Message::text("connected")).await;
	while let Some(Ok(msg)) = socket.next().await {
		match msg {
			Message::Text(_t) => {
                if let Ok(req) = serde_json::from_str::<SubscribeMsg>(&_t) {
                    if let Ok(pk) = Pubkey::from_str(&req.subscribe) {
                        let ws_url = state.cfg.solana_rpc_url.replace("https://", "wss://").replace("http://", "ws://");
                        let mut ws_sender = socket.clone();
                        tokio::spawn(async move {
                            if let Ok((mut client, mut stream)) = PubsubClient::new(&ws_url).await.and_then(|c| async move {
                                let (sub, sub_stream) = c.account_subscribe(&pk, Some(RpcAccountInfoConfig::default())).await?;
                                Ok::<_, solana_client::client_error::ClientError>((sub, sub_stream))
                            }).await {
                                while let Some(update) = stream.next().await {
                                    if let Ok(update) = update {
                                        let _ = ws_sender.send(Message::text(serde_json::json!({
                                            "pubkey": pk.to_string(),
                                            "slot": update.context.slot,
                                            "data_len": update.value.data.len(),
                                        }).to_string())).await;
                                    }
                                }
                                let _ = client.unsubscribe().await;
                            }
                        });
                        let _ = socket.send(Message::text("subscribed")).await;
                        continue;
                    }
                }
                let _ = socket.send(Message::text("pong")).await;
			}
			Message::Close(_) => break,
			_ => {}
		}
	}
}


