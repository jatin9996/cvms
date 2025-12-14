use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use futures::{stream::SplitSink, SinkExt, StreamExt};
use serde::Deserialize;
use solana_account_decoder::UiAccountData;
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::rpc_config::RpcAccountInfoConfig;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::AppState;

pub async fn ws_handler(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(state, socket))
}

#[derive(Deserialize)]
struct SubscribeMsg {
    subscribe: String,
    topic: Option<String>,
    owner: Option<String>,
}

async fn handle_socket(state: AppState, socket: WebSocket) {
    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(Mutex::new(sender));
    let _ = send_text(&sender, "connected".into()).await;
    while let Some(Ok(msg)) = receiver.next().await {
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
                            "timelock_event" => state.notifier.timelock_tx.subscribe(),
                            "vault_balance_update" => state.notifier.vault_balance_tx.subscribe(),
                            "tvl_update" => state.notifier.tvl_tx.subscribe(),
                            "security_alert" => state.notifier.security_tx.subscribe(),
                            "analytics_update" => state.notifier.analytics_tx.subscribe(),
                            _ => {
                                let _ = send_text(&sender, "unknown topic".into()).await;
                                continue;
                            }
                        };
                        let ws_sender = sender.clone();
                        let owner_filter = req.owner.clone();
                        tokio::spawn(async move {
                            while let Ok(msg) = rx.recv().await {
                                if let Some(ref owner) = owner_filter {
                                    if !msg.contains(owner) {
                                        continue;
                                    }
                                }
                                let _ = send_text(&ws_sender, msg).await;
                            }
                        });
                        let _ = send_text(&sender, "subscribed".into()).await;
                        continue;
                    }
                    if let Ok(pk) = Pubkey::from_str(&req.subscribe) {
                        let ws_url = state
                            .cfg
                            .solana_rpc_url
                            .replace("https://", "wss://")
                            .replace("http://", "ws://");
                        let ws_sender = sender.clone();
                        tokio::spawn(async move {
                            if let Ok(client) = PubsubClient::new(&ws_url).await {
                                if let Ok((mut stream, subscription)) = client
                                    .account_subscribe(&pk, Some(RpcAccountInfoConfig::default()))
                                    .await
                                {
                                    while let Some(update) = stream.next().await {
                                        let data_len = match &update.value.data {
                                            UiAccountData::Binary(data, _)
                                            | UiAccountData::LegacyBinary(data) => BASE64_STANDARD
                                                .decode(data)
                                                .map(|bytes| bytes.len())
                                                .unwrap_or_else(|_| data.len()),
                                            _ => 0,
                                        };
                                        let _ = send_text(
                                            &ws_sender,
                                            serde_json::json!({
                                                "pubkey": pk.to_string(),
                                                "slot": update.context.slot,
                                                "data_len": data_len,
                                            })
                                            .to_string(),
                                        )
                                        .await;
                                    }
                                    let _ = subscription().await;
                                }
                            }
                        });
                        let _ = send_text(&sender, "subscribed".into()).await;
                        continue;
                    }
                }
                let _ = send_text(&sender, "pong".into()).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

async fn send_text(
    sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    text: String,
) -> Result<(), ()> {
    let mut guard = sender.lock().await;
    guard.send(Message::Text(text)).await.map_err(|_| ())
}
