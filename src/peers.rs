use std::path::{Path, PathBuf};

use ipc_channel::ipc::IpcSender;
use uuid::Uuid;

use crate::{BusRider, Handle};

trait NeighborManager {}

trait Neighbor {
    fn get_uuid(&self) -> Uuid;
}

struct IpcPeer {
    uuid: Uuid,
}

impl Neighbor for IpcPeer {
    fn get_uuid(&self) -> Uuid {
        self.uuid
    }
}

pub(crate) struct IpcManager {
    rendezvous: PathBuf,
    handle: Handle,
}

impl IpcManager {
    pub(crate) fn new(rendezvous: PathBuf, handle: Handle) -> Self {
        IpcManager { rendezvous, handle }
    }

    pub(crate) async fn start(self) {}
}

impl NeighborManager for IpcManager {}

enum IpcMessage {
    NeighborAdded(Uuid, IpcSender<IpcMessage>),
    NeighborRemoved(Uuid),
    BusRider(Uuid, Box<dyn BusRider>),
}
