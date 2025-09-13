pub(super) mod ipc_manager;
mod ipc_peer;

use std::collections::HashSet;

use async_bincode::{AsyncDestination, tokio::AsyncBincodeStream};
use interprocess::local_socket::{GenericNamespaced, Name, ToNsName};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::routing::{Address, Advertisement, NodeId, WirePacket};

pub(super) type IpcPeerStream = AsyncBincodeStream<
    interprocess::local_socket::tokio::Stream,
    IpcMessage,
    IpcMessage,
    AsyncDestination,
>;

/// Helper trait to convert Uuid to a 'interprocess' Name<>
pub(super) trait NameHelper {
    fn to_name(&self) -> Name<'static>;
}
impl NameHelper for Uuid {
    fn to_name(&self) -> Name<'static> {
        format!("anybus.ipc.{}", self)
            .to_ns_name::<GenericNamespaced>()
            .unwrap()
    }
}

#[derive(Debug)]
pub(super) enum IpcCommand {
    // AddPeer(Uuid, PeerTx, PeerRx, bool), // bool is if the peer was found by the discovery agent
    PeerClosed(Uuid, bool),
    LearnedPeers(Vec<Uuid>),
}

#[derive(Debug)]
pub(super) enum IpcControl {
    IAmMaster,
    Shutdown,
}

/// Protocol messages for the IPC bus.
#[derive(Serialize, Deserialize)]
pub(super) enum IpcMessage {
    Hello(NodeId), //Our AnyBus ID
    KnownPeers(Vec<NodeId>),
    NeighborRemoved(NodeId), //Node/Peer ID
    // BusRider(Address, Vec<u8>), // Destination ID
    CloseConnection,
    Advertise(HashSet<Advertisement>),
    Withdraw(HashSet<Advertisement>),
    Packet(WirePacket),
    IAmMaster,
}

impl std::fmt::Debug for IpcMessage {
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
            // IpcMessage::BusRider(uuid, bytes) => {
            //     write!(f, "BusRider({}, {} bytes)", uuid, bytes.len())
            // }
            IpcMessage::CloseConnection => write!(f, "CloseConnection"),
            IpcMessage::Advertise(ads) => write!(f, "Advertise({:?})", ads),
            IpcMessage::IAmMaster => write!(f, "IAmMaster"),
            IpcMessage::Withdraw(uuids) => write!(f, "Withdraw ({:?})", uuids),
            IpcMessage::Packet(_wire_packet) => write!(f, "Packet(..)"),
        }
    }
}
