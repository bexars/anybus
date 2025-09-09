use std::sync::Arc;

use tokio::sync::{
    RwLock,
    mpsc::{UnboundedReceiver, UnboundedSender},
    watch,
};
use uuid::Uuid;

use crate::{
    Handle,
    messages::BusControlMsg,
    peers::ws::{WsCommand, WsControl},
};

struct WebsocketManager {
    handle: Handle,
    bus_control: watch::Receiver<BusControlMsg>,
    peers: Arc<RwLock<Vec<(Uuid, UnboundedSender<WsControl>)>>>,
    tx: UnboundedSender<WsCommand>,
    rx: UnboundedReceiver<WsCommand>,
    uuid: Uuid,
}
