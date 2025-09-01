mod ipc;
mod ipc_manager;
use core::fmt;

pub(crate) use ipc_manager::IpcManager;

use async_bincode::{AsyncDestination, tokio::AsyncBincodeStream};
use interprocess::local_socket::{GenericNamespaced, Name, ToNsName};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

type IpcPeerStream = AsyncBincodeStream<
    interprocess::local_socket::tokio::Stream,
    IpcMessage,
    IpcMessage,
    AsyncDestination,
>;

use crate::route_table::Advertisement;

/// Helper trait to convert Uuid to a 'interprocess' Name<>
trait NameHelper {
    fn to_name(&self) -> Name<'static>;
}
impl NameHelper for Uuid {
    fn to_name(&self) -> Name<'static> {
        format!("msgbus.ipc.{}", self)
            .to_ns_name::<GenericNamespaced>()
            .unwrap()
    }
}

#[derive(Debug)]
enum IpcCommand {
    // AddPeer(Uuid, PeerTx, PeerRx, bool), // bool is if the peer was found by the discovery agent
    PeerClosed(Uuid, bool),
    LearnedPeers(Vec<Uuid>),
}

#[derive(Debug)]
enum IpcControl {
    IAmMaster,
    Shutdown,
}

/// Protocol messages for the IPC bus.
#[derive(Serialize, Deserialize)]
enum IpcMessage {
    Hello(Uuid), //Our Msgbus ID
    KnownPeers(Vec<Uuid>),
    NeighborRemoved(Uuid),   //Node/Peer ID
    BusRider(Uuid, Vec<u8>), // Destination ID
    CloseConnection,
    Advertise(Vec<Advertisement>),
    IAmMaster,
}

impl fmt::Debug for IpcMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // f.debug_struct("IpcMessage").
        match self {
            IpcMessage::Hello(uuid) => write!(f, "Hello({})", uuid),
            IpcMessage::KnownPeers(uuids) => {
                write!(f, "KnownPeers({:?})", uuids)
            }
            IpcMessage::NeighborRemoved(uuid) => {
                write!(f, "NeighborRemoved({})", uuid)
            }
            IpcMessage::BusRider(uuid, bytes) => {
                write!(f, "BusRider({}, {} bytes)", uuid, bytes.len())
            }
            IpcMessage::CloseConnection => write!(f, "CloseConnection"),
            IpcMessage::Advertise(ads) => write!(f, "Advertise({:?})", ads),
            IpcMessage::IAmMaster => write!(f, "IAmMaster"),
        }
    }
}
