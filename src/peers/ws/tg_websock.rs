use futures::{SinkExt, StreamExt};

use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream as ServerTlsStream;

// use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};
use tracing::error;
// use tungstenite::stream::MaybeTlsStream;

use crate::peers::ws::{WsMessage, ws_peer::InMessage};
// From<tokio_tungstenite::WebSocketStream<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>>

pub enum WebSockStream {
    TgServer(WebSocketStream<ServerTlsStream<TcpStream>>),
    TgClient(WebSocketStream<MaybeTlsStream<TcpStream>>),
}

impl WebSockStream {
    pub(crate) async fn send_msg(
        &mut self,
        message: super::WsMessage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            WebSockStream::TgServer(s) => Ok(s.send(Message::Binary(message.into())).await?),
            WebSockStream::TgClient(s) => Ok(s.send(Message::Binary(message.into())).await?),
        }
    }

    pub(crate) async fn next_msg(&mut self) -> InMessage {
        let res: Option<Result<Message, tokio_tungstenite::tungstenite::Error>> = match self {
            WebSockStream::TgServer(s) => s.next().await,
            WebSockStream::TgClient(s) => s.next().await,
        };
        res.into()
    }

    pub(crate) async fn close_conn(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let res = match self {
            WebSockStream::TgServer(s) => s.close(None).await,
            WebSockStream::TgClient(s) => s.close(None).await,
        };
        Ok(res?)
    }
}

// impl From<WebSocketStream<ServerTlsStream<TcpStream>>> for WebSockStream {
//     fn from(value: WebSocketStream<ServerTlsStream<TcpStream>>) -> Self {
//         Self::TgServer(value)
//     }
// }

impl From<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>
    for WebSockStream
{
    fn from(
        value: WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ) -> Self {
        Self::TgClient(value)
    }
}

impl From<WebSocketStream<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>>
    for WebSockStream
{
    fn from(
        value: WebSocketStream<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>,
    ) -> Self {
        Self::TgServer(value)
    }
}

// impl From<WebSocketStream<ClientTlsStream<TcpStream>>> for WebSockStream {
//     fn from(value: WebSocketStream<ClientTlsStream<TcpStream>>) -> Self {
//         Self::TgClient(value)
//     }
// }

impl From<Option<Result<Message, tokio_tungstenite::tungstenite::Error>>> for InMessage {
    fn from(msg: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>) -> Self {
        match msg {
            Some(Ok(message)) => {
                match message {
                    Message::Binary(data) => {
                        let msg = serde_cbor::from_slice::<WsMessage>(&data)
                            .map(|msg| InMessage::WsMessage(msg))
                            .unwrap_or_else(|e| {
                                error!("Failed to deserialize WsMessage: {}", e);
                                InMessage::WsPeerClosed
                            });
                        msg

                        // Some(InMessage::WsMessage(data))
                    }
                    Message::Close(_) => InMessage::WsPeerClosed,
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
