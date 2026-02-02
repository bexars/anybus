use std::sync::Arc;

use futures::{Sink, SinkExt, Stream, StreamExt};

use tokio::sync::Mutex;
// use tokio_tungstenite::MaybeTlsStream;
use tracing::error;
use wasm_socket_handle::{WsError, WsHandle, WsMessage as WsHandleMessage};

// use tungstenite::stream::MaybeTlsStream;

use crate::peers::ws::{WsMessage, ws_peer::InMessage};
// From<tokio_tungstenite::WebSocketStream<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>>

pub enum WebSockStream {
    Wasm(Arc<Mutex<WsHandle>>),
}

impl WebSockStream {
    pub(crate) async fn send_msg(
        &mut self,
        message: super::WsMessage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            WebSockStream::Wasm(s) => Ok(s
                .lock()
                .await
                .send(WsHandleMessage::Binary(message.into()))
                .await?),
        }
    }

    pub(crate) async fn next_msg(&mut self) -> InMessage {
        let res = match self {
            WebSockStream::Wasm(s) => s.lock().await.next().await,
        };
        res.into()
    }

    pub(crate) async fn close_conn(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let res = match self {
            WebSockStream::Wasm(s) => s.lock().await.close(),
        };
        Ok(res?)
    }
}

// impl From<WebSocketStream<ServerTlsStream<TcpStream>>> for WebSockStream {
//     fn from(value: WebSocketStream<ServerTlsStream<TcpStream>>) -> Self {
//         Self::TgServer(value)
//     }
// }

impl From<WsHandle> for WebSockStream {
    fn from(value: WsHandle) -> Self {
        Self::Wasm(Arc::new(Mutex::new(value)))
    }
}

impl From<Option<Result<WsHandleMessage, WsError>>> for InMessage {
    fn from(msg: Option<Result<WsHandleMessage, WsError>>) -> Self {
        match msg {
            Some(Ok(message)) => {
                match message {
                    WsHandleMessage::Binary(data) => {
                        let msg = serde_cbor::from_slice::<WsMessage>(&data)
                            .map(|msg| InMessage::WsMessage(msg))
                            .unwrap_or_else(|e| {
                                error!("Failed to deserialize WsMessage: {}", e);
                                InMessage::WsPeerClosed
                            });
                        msg

                        // Some(InMessage::WsMessage(data))
                    }
                    // WsHandleMessage::Close(_) => InMessage::WsPeerClosed,
                    _ => InMessage::Unknown, // Ignore other message types for now
                }
            }
            Some(Err(e)) => {
                error!("WebSocket error: {}", e);
                InMessage::WsPeerClosed
            }
            None => {
                // Connection closed
                InMessage::WsPeerClosed
            }
        }
    }
}
