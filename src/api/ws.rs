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
struct SubscribeMsg { subscribe: String, topic: Option<String>, owner: Option<String> }

async fn handle_socket(state: AppState, mut socket: WebSocket) {
	let _ = socket.send(Message::text("connected")).await;
    while let Some(Ok(msg)) = socket.next().await {
		match msg {
			Message::Text(_t) => {
                if let Ok(req) = serde_json::from_str::<SubscribeMsg>(&_t) {
                    // Topic-based subscription via notifier
                    if let Some(topic) = req.topic.as_ref() {
                        let mut rx = match topic.as_str() {
                            "deposit_event" => state.notifier.deposit_tx.subscribe(),
                            "withdraw_event" => state.notifier.withdraw_tx.subscribe(),
                            "lock_event" => state.notifier.lock_tx.subscribe(),
                            "unlock_event" => state.notifier.unlock_tx.subscribe(),
                            "vault_balance_update" => state.notifier.vault_balance_tx.subscribe(),
                            "tvl_update" => state.notifier.tvl_tx.subscribe(),
						"security_alert" => state.notifier.security_tx.subscribe(),
						"analytics_update" => state.notifier.analytics_tx.subscribe(),
                            _ => {
                                let _ = socket.send(Message::text("unknown topic"));
                                continue;
                            }
                        };
                        let mut ws_sender = socket.clone();
                        let owner_filter = req.owner.clone();
                        tokio::spawn(async move {
                            while let Ok(msg) = rx.recv().await {
                                if let Some(ref owner) = owner_filter {
                                    if !msg.contains(owner) { continue; }
                                }
                                let _ = ws_sender.send(Message::text(msg)).await;
                            }
                        });
                        let _ = socket.send(Message::text("subscribed")).await;
                        continue;
                    }
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


