mod ipc;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

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
        RwLock,
        mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
        watch,
    },
};
use tracing::error;
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
    peers::ipc::IpcPeer,
    spawn,
};

// struct IpcPeer {
//     // tx: PeerTx,
//     rx: PeerRx,
//     ipc_command: UnboundedSender<IpcCommand>,
//     peer: Peer,
// }

// impl IpcPeer {
//     pub(crate) fn start(self) {

//         // Start the IpcPeer
//     }
// }

pub(crate) struct IpcManager {
    rendezvous: PathBuf,
    handle: Handle,
    bus_control: watch::Receiver<BusControlMsg>,
    peers: Arc<RwLock<Vec<(Uuid, UnboundedSender<IpcControl>)>>>,
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
        let (tx, rx) = unbounded_channel();

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
                Some(msg) = self.rx.recv() => { self.handle_message(msg).await; },
                _ = self.bus_control.changed() => {
                    //TODO evaluate the message and then die
                    break;
                }
            }
        }
        self.peers.read().await.iter().for_each(|(_id, tx)| {
            let _ = tx.send(IpcControl::Shutdown);
        });
    }

    async fn handle_message(&mut self, msg: IpcCommand) {
        //
        match msg {
            IpcCommand::AddPeer(uuid, ipc_sender, ipc_receiver) => {
                let (tx, rx) = unbounded_channel();
                let (tx_ipc_control, rx_ipc_control) = unbounded_channel();

                let peer = Peer {
                    uuid,
                    rx,
                    handle: self.handle.clone(),
                };

                let ipc_peer = IpcPeer::new(
                    ipc_receiver,
                    ipc_sender,
                    self.tx.clone(),
                    rx_ipc_control,
                    self.peers.clone(),
                    peer,
                );
                _ = spawn(ipc_peer.start());
                self.peers.write().await.push((uuid, tx_ipc_control));
                self.handle.register_peer(uuid, tx);
                // spawn(ipc_peer.start());
            }
            IpcCommand::PeerClosed(uuid) => {
                self.peers
                    .write()
                    .await
                    .retain(|(id, _tx)| if *id == uuid { false } else { true });
            }
        }
    }
}

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
                        _ = self.tx.send(IpcCommand::AddPeer(other_uuid, tx, rx));
                    }
                    //Otherwise fallthrough and close the task
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
                ipc_listener(self.uuid, Some(name), self.tx.clone(), self.bus_control).await;
            }
        }
        // TODO loop to check for rendezvous server and start if it's not running
    }
    // async fn launch_rendezvous_server(&self, name: Name<'_>) {
    //     dbg!(&name);
    //     let listener_opts = local_socket::ListenerOptions::new()
    //         .nonblocking(local_socket::ListenerNonblockingMode::Neither)
    //         .name(name)
    //         .reclaim_name(true);
    //     let res = listener_opts.create_tokio().unwrap();
    //     loop {
    //         let client = res.accept().await.unwrap();

    //         let mut stream = AsyncBincodeStream::from(client).for_async();

    //         if let Some(msg) = stream.next().await {
    //             if let Ok(IpcMessage::Hello(other_uuid)) = msg
    //                 && other_uuid != self.uuid
    //             {
    //                 stream.send(IpcMessage::Hello(self.uuid)).await.unwrap();
    //                 let (tx, rx) = stream.split();
    //                 _ = self.tx.send(IpcCommand::AddPeer(other_uuid, tx, rx));
    //             } else {
    //                 _ = stream.close();
    //             };
    //         }
    //     }
    // }
}

/// Helper trait to convert Uuid to a 'interprocess' Name<>
trait NameHelper {
    fn to_name(&self) -> Name<'_>;
}
impl NameHelper for Uuid {
    fn to_name(&self) -> Name<'_> {
        format!("msgbus.ipc.{}", self)
            .to_ns_name::<GenericNamespaced>()
            .unwrap()
    }
}

async fn ipc_listener(
    uuid: Uuid,
    name: Option<Name<'_>>,
    cmd_tx: UnboundedSender<IpcCommand>,
    mut bus_control: watch::Receiver<BusControlMsg>,
) {
    let name = name.unwrap_or_else(|| uuid.to_name());

    dbg!(&name);
    let listener_opts = local_socket::ListenerOptions::new()
        .nonblocking(local_socket::ListenerNonblockingMode::Neither)
        .name(name)
        .reclaim_name(true);
    let res = listener_opts.create_tokio().unwrap();
    loop {
        select! {
            _ = bus_control.changed() => {
                //TODO evaluate the message and then die
                break;
            },
            accept_res = res.accept() => {
                if let Err(e) = accept_res {
                    error!("Error accepting IPC connection: {:?}", e);
                    continue;
                }
                let client = accept_res.unwrap();
                let mut stream = AsyncBincodeStream::from(client).for_async();

                if let Some(msg) = stream.next().await {
                    if let Ok(IpcMessage::Hello(other_uuid)) = msg
                        && other_uuid != uuid
                    {
                        stream.send(IpcMessage::Hello(uuid)).await.unwrap();
                        let (tx, rx) = stream.split();
                        _ = cmd_tx.send(IpcCommand::AddPeer(other_uuid, tx, rx));
                    } else {
                        _ = stream.close();
                    };
                }
            }
        }
    }
}

enum IpcCommand {
    AddPeer(Uuid, PeerTx, PeerRx),
    PeerClosed(Uuid),
}

enum IpcControl {
    Shutdown,
    PeersChanged,
}

/// Protocol messages for the IPC bus.
#[derive(Debug, Serialize, Deserialize)]
enum IpcMessage {
    Hello(Uuid), //Our Msgbus ID
    KnownPeers(Vec<Uuid>),
    NeighborRemoved(Uuid),   //Node/Peer ID
    BusRider(Uuid, Vec<u8>), // Destination ID
}
