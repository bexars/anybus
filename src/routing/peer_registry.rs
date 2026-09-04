use std::{collections::HashMap, slice::IterMut};

use crate::routing::{NodeId, router::PeerInfo};

#[derive(Default)]
pub(crate) struct PeerRegistry {
    // The master storage
    peers: Vec<PeerInfo>,
    // Indexes pointing back to the vector slots
    by_node_id: HashMap<NodeId, usize>,
    by_connection_id: HashMap<u16, usize>,
}

impl PeerRegistry {
    pub fn insert(&mut self, peer: PeerInfo) {
        let index = self.peers.len();
        self.by_node_id.insert(peer.peer_id, index);
        self.by_connection_id.insert(peer.connection_id, index);
        self.peers.push(peer);
    }

    pub fn remove_by_connection_id(&mut self, id: u16) -> Option<PeerInfo> {
        // 1. Find the target index
        let target_idx = self.by_connection_id.remove(&id)?;

        // 2. Remove the email index entry for this user
        let peer_to_remove = &self.peers[target_idx];
        self.by_node_id.remove(&peer_to_remove.peer_id);

        // 3. Use swap_remove to take the element out in O(1) time
        let removed_user = self.peers.swap_remove(target_idx);

        // 4. CRITICAL: If the target wasn't the last element, another element
        // was just moved into `target_idx`. We must update its map pointers!
        if target_idx < self.peers.len() {
            let moved_peer = &self.peers[target_idx];
            self.by_connection_id
                .insert(moved_peer.connection_id, target_idx);
            self.by_node_id.insert(moved_peer.peer_id, target_idx);
        }

        Some(removed_user)
    }

    #[allow(unused)]
    pub fn get_by_connection_id(&self, id: u16) -> Option<&PeerInfo> {
        self.by_connection_id.get(&id).map(|&idx| &self.peers[idx])
    }

    #[allow(unused)]
    pub fn get_by_node_id(&self, peer_id: &NodeId) -> Option<&PeerInfo> {
        self.by_node_id.get(peer_id).map(|&idx| &self.peers[idx])
    }

    pub fn get_mut_by_connection_id(&mut self, id: u16) -> Option<&mut PeerInfo> {
        self.by_connection_id
            .get(&id)
            .map(|&idx| &mut self.peers[idx])
    }
    #[allow(unused)]
    pub fn get_mut_by_node_id(&mut self, id: NodeId) -> Option<&mut PeerInfo> {
        self.by_node_id.get(&id).map(|&idx| &mut self.peers[idx])
    }
    #[allow(unused)]
    pub fn contains_node_id_key(&self, node_id: &NodeId) -> bool {
        self.by_node_id.contains_key(node_id)
    }

    pub fn contains_connection_id_key(&self, connection_id: u16) -> bool {
        self.by_connection_id.contains_key(&connection_id)
    }
    #[allow(unused)]
    pub fn iter(&self) -> impl Iterator<Item = &PeerInfo> {
        self.peers.iter()
    }

    pub fn iter_mut<'a>(&'a mut self) -> IterMut<'a, PeerInfo> {
        self.peers.iter_mut()
    }

    pub fn values(&self) -> impl Iterator<Item = &PeerInfo> {
        self.peers.iter()
    }
}
