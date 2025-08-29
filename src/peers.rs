use std::path::{Path, PathBuf};

use async_bincode::{AsyncDestination, tokio::AsyncBincodeStream};
use futures::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use interprocess::local_socket::{
    self, GenericNamespaced, Name, ToNsName,
    traits::tokio::{Listener, Stream},
};

use serde::{Deserialize, Serialize};
use tokio::{
    select,
    sync::{
        mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
        watch,
    },
};
use uuid::Uuid;
type PeerTx = SplitSink<
    AsyncBincodeStream<
        interprocess::local_socket::tokio::Stream,
        IpcMessage,
        IpcMessage,
        AsyncDestination,
    >,
    IpcMessage,
>;
type PeerRx = SplitStream<
    AsyncBincodeStream<
        interprocess::local_socket::tokio::Stream,
        IpcMessage,
        IpcMessage,
        AsyncDestination,
    >,
>;

use crate::{
    BusRider, Handle, Node, Peer,
    messages::{BusControlMsg, NodeMessage},
    spawn,
};

trait NeighborManager {}

trait Neighbor {
    fn get_uuid(&self) -> Uuid;
}

struct IpcPeer {
    // tx: PeerTx,
    rx: PeerRx,
    ipc_command: UnboundedSender<IpcCommand>,
    peer: Peer,
}

impl IpcPeer {
    pub(crate) fn start(self) {

        // Start the IpcPeer
    }
}

impl Neighbor for IpcPeer {
    fn get_uuid(&self) -> Uuid {
        self.peer.uuid
    }
}

pub(crate) struct IpcManager {
    rendezvous: PathBuf,
    handle: Handle,
    bus_control: watch::Receiver<BusControlMsg>,
    peers: Vec<(Uuid, PeerTx)>,
    tx: UnboundedSender<IpcCommand>,
    rx: UnboundedReceiver<IpcCommand>,
    uuid: Uuid,
}

impl IpcManager {
    pub(crate) fn new(
        rendezvous: PathBuf,
        handle: Handle,
        bus_control: watch::Receiver<BusControlMsg>,
        uuid: Uuid,
    ) -> Self {
        let (tx, mut rx) = unbounded_channel();

        IpcManager {
            rendezvous,
            handle,
            bus_control,
            peers: Default::default(),
            tx,
            rx,
            uuid,
        }
    }

    pub(crate) async fn start(mut self) {
        let agent = IpcDiscoveryAgent {
            tx: self.tx.clone(),
            bus_control: self.bus_control.clone(),
            rendezvous: "msgbus.ipc".to_string(),
            uuid: self.uuid,
        };
        spawn(agent.start());

        loop {
            select! {
                Some(msg) = self.rx.recv() => { self.handle_message(msg); },
                _ = self.bus_control.changed() => {
                    //TODO evaluate the message and then die
                    break;
                }
            }
        }
        //TODO send shutdown message to all neighbors
    }

    fn handle_message(&mut self, msg: IpcCommand) {
        //
        match msg {
            IpcCommand::AddPeer(uuid, ipc_sender, ipc_receiver) => {
                let (tx, rx) = unbounded_channel();

                let peer = Peer {
                    uuid,
                    rx,
                    handle: self.handle.clone(),
                };

                let ipc_peer = IpcPeer {
                    peer,
                    // tx: ipc_sender,
                    rx: ipc_receiver,
                    ipc_command: self.tx.clone(),
                };
                self.peers.push((uuid, ipc_sender));
                self.handle.register_peer(uuid, tx.clone());
                ipc_peer.start();
            }
        }
    }
}

impl NeighborManager for IpcManager {}

#[derive(Debug, Clone)]
struct IpcDiscoveryAgent {
    tx: UnboundedSender<IpcCommand>,
    bus_control: watch::Receiver<BusControlMsg>,
    rendezvous: String,
    uuid: Uuid,
}
impl IpcDiscoveryAgent {
    async fn start(self) {
        let name = self
            .rendezvous
            .clone()
            .to_ns_name::<GenericNamespaced>()
            .unwrap();
        let stream = local_socket::tokio::Stream::connect(name.clone()).await;
        match stream {
            Ok(stream) => {
                // let (rx, tx) = stream.split();

                let stream: AsyncBincodeStream<
                    local_socket::tokio::Stream,
                    IpcMessage,
                    IpcMessage,
                    async_bincode::AsyncDestination,
                > = AsyncBincodeStream::from(stream).for_async();

                let (mut tx, mut rx) = stream.split();

                let msg = IpcMessage::Hello(self.uuid);
                dbg!(&msg);
                tx.send(msg).await.unwrap();
                let msg = rx.next().await.unwrap().unwrap();
                dbg!(&msg);
                if let IpcMessage::Hello(other_uuid) = msg {
                    if self.uuid != other_uuid {
                        // Handle the case where the UUIDs match
                        self.tx.send(IpcCommand::AddPeer(other_uuid, tx, rx));
                    }
                }
                // self.tx.send(IpcCommand::AddPeer((), (), ()))
            }
            Err(_) => {
                // Handle the error
                #[cfg(unix)]
                let _ = {
                    let path = PathBuf::new().join("/tmp").join(&self.rendezvous);
                    _ = std::fs::remove_file(path);
                };
                self.launch_rendezvous_server(name).await;
            }
        }
        // TODO loop to check for rendezvous server and start if it's not running
    }
    async fn launch_rendezvous_server(&self, name: Name<'_>) {
        dbg!(&name);
        let listener_opts = local_socket::ListenerOptions::new()
            .nonblocking(local_socket::ListenerNonblockingMode::Neither)
            .name(name)
            .reclaim_name(true);
        let res = listener_opts.create_tokio().unwrap();
        loop {
            let client = res.accept().await.unwrap();

            let mut stream = AsyncBincodeStream::from(client).for_async();

            while let Some(msg) = stream.next().await {
                if let Ok(IpcMessage::Hello(peer_uuid)) = msg {
                    stream.send(IpcMessage::Hello(self.uuid)).await.unwrap();
                } else {
                    break;
                }
            }
        }
    }
}

enum IpcCommand {
    AddPeer(Uuid, PeerTx, PeerRx),
}

/// Protocol messages for the IPC bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
enum IpcMessage {
    Hello(Uuid),
    NeighborRemoved(Uuid),
    NodeMsg(Vec<u8>),
}
