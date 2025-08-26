use std::panic;

use if_addrs::{IfAddr, Ifv6Addr, Interface};
use uuid::Uuid;

use crate::{neighbor::NeighborDiscovery, BusControlMsg};

#[allow(unused_variables)]
#[allow(dead_code)]

pub struct Router {
    interfaces: Vec<InterfaceListener>,
    id: Uuid,
    bcs_rx: tokio::sync::watch::Receiver<BusControlMsg>,
}

impl Router {
    pub(crate) fn new(id: Uuid, bcs_rx: tokio::sync::watch::Receiver<BusControlMsg>) -> Self {
        Self {
            interfaces: Vec::new(),
            id,
            bcs_rx,
        }
    }

    pub(crate) async fn run(self) {
        let addrs = if_addrs::get_if_addrs().unwrap();
        let link_locals: Vec<_> = addrs
            .into_iter() // TODO have a smarter filtering system with external input; ENV vars?
            .filter(|a| {
                if let IfAddr::V6(_b) = &a.addr {
                    true
                } else {
                    false
                }
            })
            .filter(|a| a.addr.is_link_local())
            .filter(|a| !a.name.starts_with("lo"))
            .filter(|a| a.name.contains("en0"))
            .map(|int| {
                let IfAddr::V6(Ifv6Addr {
                    ip,
                    netmask: _,
                    broadcast: _,
                }) = int.addr
                else {
                    panic!()
                };
                let Some(index) = int.index else { panic!() };
                (ip, index, int.name)
            })
            .collect();
        for (ip_addr, int_index, name) in link_locals {
            let nd = NeighborDiscovery::new(self.id, self.bcs_rx.clone(), ip_addr, int_index, name)
                .await;
            tokio::spawn(nd.run());
        }
    }
}

#[allow(dead_code)]
struct InterfaceListener {
    interface: Interface,
    listener: NeighborDiscovery,
}
