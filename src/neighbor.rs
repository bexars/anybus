use lazy_static::*;
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;

use std::io::{self, Cursor};
use std::net::SocketAddrV6;
use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr},
    str::FromStr,
    time::Duration,
};
use tokio::time::{Instant, MissedTickBehavior};
use tokio::{net::UdpSocket, select, time};

use uuid::Uuid;

use crate::BusControlMsg;

use binrw::{
    binrw, // #[binrw] attribute
    helpers::count_with,
    BinRead,   // trait for reading
    BinResult, // trait for writing
    BinWrite,
};

const VERSION: u8 = 1;
// const MAGIC: u8 = 109; // 'm' for Msgbus  // Weirdly can't use this in brw(magic)

#[allow(dead_code)]
struct NDContext {
    all_socket: SocketAddr,
    dr_socket: SocketAddr,
    socket: UdpSocket,
    local_port: u16,
    local_addr: Ipv6Addr,
}


#[allow(dead_code)]
pub(crate) struct NeighborDiscovery {
    id: Uuid,
    state: NDState,
    bcs_rx: tokio::sync::watch::Receiver<BusControlMsg>,
    ip_addr: Ipv6Addr,
    neighbors: HashMap<Uuid, Neighbor>,
    int_index: u32,
    dr: Option<Uuid>,
    bdr: Option<Uuid>,
    context: NDContext,
}

lazy_static! {
    // pub static ref IPV4: IpAddr = Ipv4Addr::new(224, 0, 0, 123).into();
    pub static ref LISTEN_ADDRESS: Ipv6Addr = Ipv6Addr::new(0xFF02, 0, 0, 0, 0, 0, 0, 0x0123).into();
}

#[allow(dead_code)]
const LISTEN_PORT: u16 = 3069;

impl NeighborDiscovery {
    pub(crate) async fn new(
        id: Uuid,
        bcs_rx: tokio::sync::watch::Receiver<BusControlMsg>,
        ip_addr: Ipv6Addr,
        int_index: u32,
        _name: String,
    ) -> Self {
        let state = {
            let state: tokio::sync::watch::Ref<'_, BusControlMsg> = bcs_rx.borrow();

            match *state {
                BusControlMsg::Run => NDState::Idle,
                BusControlMsg::Shutdown => NDState::Down,
            }
        };
        // dbg!(&state, "About to init_socket()");
        let (socket, port) = Self::init_socket(ip_addr, int_index).await;

        // dbg!("Returned Socket", &socket);
        let neighbors = HashMap::new();
        // let interface = 11; // TODO replace this with learned interface

        let mut nd_mult_addr = SocketAddrV6::from_str("[ff02::123]:3069").unwrap();
        nd_mult_addr.set_scope_id(int_index);
        let nd_mult_addr = SocketAddr::V6(nd_mult_addr);
        let mut dr_mult_addr = SocketAddrV6::from_str("[ff02::123]:3069").unwrap();
        dr_mult_addr.set_scope_id(int_index);
        let dr_mult_addr = SocketAddr::V6(dr_mult_addr);

        let context = NDContext {
            socket,
            all_socket: nd_mult_addr,
            dr_socket: dr_mult_addr,
            local_port: port,
            local_addr: ip_addr,
        };

        Self {
            id,
            state,
            bcs_rx,
            ip_addr,
            neighbors,
            int_index,
            dr: None,
            bdr: None,
            context,
        }
    }

    async fn init_socket(ip_addr: Ipv6Addr, int_index: u32) -> (UdpSocket, u16) {
        let random_addr: SocketAddr = SocketAddrV6::new(ip_addr, 0, 0, int_index).into();
        let socket = tokio::net::UdpSocket::bind(random_addr).await.unwrap();

        let sock_addr = socket.local_addr().unwrap();
        dbg!(&sock_addr);
        // let IpAddr::V6(ip_addr) = sock_addr.ip() else { panic!() };
        let port = sock_addr.port();
        (socket, port)
    }

    pub(crate) async fn run(mut self) {
        //  let nd_mult_addr = SocketAddr::new(IpAddr::V6(*LISTEN_ADDRESS), LISTEN_PORT);

        let _dr: Option<Uuid> = None;
        let _bdr: Option<Uuid> = None;

        let mut buf = vec![0u8; 9000];
        let mut buf2 = vec![0u8; 9000];

        self.context
            .socket
            .join_multicast_v6(&*LISTEN_ADDRESS, self.int_index)
            .unwrap();

        let mult_socket = join_multicast(self.context.all_socket).unwrap();

        let mut hello_timer = time::interval(Duration::from_secs(10));
        hello_timer.set_missed_tick_behavior(MissedTickBehavior::Delay); // 10 second

        loop {
            select! {
                _ = self.bcs_rx.changed() => {
                    let state = self.bcs_rx.borrow();
                    match *state {
                        BusControlMsg::Run => { continue },
                        BusControlMsg::Shutdown => {
                            self.state = NDState::Down;
                            break;
                        },
                    }

                },

                // Ok(size) = self.socket.recv(&mut buf) => {},
                Ok(size) = mult_socket.recv(&mut buf) => {

                    let mut cursor = Cursor::new(&mut buf[0..size]);
                    let msg = match NeighborMessage::read(&mut cursor) {
                        Ok(msg) => msg,
                        Err(e) => {
                            eprintln!("Neighbor message decode failure {:?}",e);  //TODO LOG
                            continue} ,
                    };
                    self.handle_message(msg).await;
                },

                Ok(size) = self.context.socket.recv(&mut buf2) => {
                    let mut cursor = Cursor::new(&mut buf2[0..size]);
                    let msg = match NeighborMessage::read(&mut cursor) {
                        Ok(msg) => msg,
                        Err(e) => {
                            eprintln!("Neighbor message decode failure {:?}",e);  //TODO LOG
                            continue} ,
                    };
                    self.handle_message(msg).await;

                },
                _now = hello_timer.tick() => {
                    match self.state {
                        NDState::Down => { continue },
                        NDState::Idle => {
                            let _ = self.send_hello(&self.context.all_socket).await;  //TODO do something with an error
                            // dbg!(&self.socket.local_addr());
                            self.state = NDState::HelloSent;
                        },
                        NDState::HelloSent => {
                            let _ = self.send_hello(&self.context.all_socket).await;  //TODO do something with an error
                        },
                        NDState::Adjacency => {},
                        NDState::DR => {},
                        NDState::BDR => {},
                    }


                        // let mut dead_list = Vec::new();
                        // for (id, n) in neighbor_map.iter() {
                        //     let diff = now - n.last_hello;
                        //     println!("{:?} ### {id}", diff);
                        //     if diff > Duration::from_secs(40) {
                        //         println!("Dead timer expired for {id}"); //TODO Make this a log output
                        //         dead_list.push(*id);
                        //     };
                        // };
                        // for id in dead_list {
                        //     neighbor_map.remove(&id);
                        // }

                }
            }
        }
        if self.state == NDState::Down {
            let shutdown_msg = NeighborMessage {
                from: self.id,
                version: VERSION,
                subtype: NeighborMessageType::Shutdown,
            };
            let mut writer = Cursor::new(Vec::new());

            shutdown_msg.write(&mut writer).unwrap();
            let _ = self.context.socket.send(writer.get_ref()).await;
        }
    }


   
    async fn handle_message(&mut self, msg: NeighborMessage) {
        if msg.from == self.id {
            return;
        }; // short-circuit the messages looped back to myself

        match msg.subtype {
            #[allow(unused_variables)]
            NeighborMessageType::Hello {
                address,
                port,
                priority,
                neighbors,
                dr,
                bdr,
            } => {
                let neighbor = self.neighbors.entry(msg.from).or_insert_with(|| Neighbor {
                    last_hello: Instant::now(),
                    state: NeighborState::Init,
                    address,
                    port,
                });

                neighbor.handle_event(NeighborEvent::HelloReceived, &self.context)
            }

            NeighborMessageType::Shutdown => todo!(),
        }
    }

    async fn _handle_neighbor_message(&mut self, msg: NeighborMessage) {
        // let Ok(msg) = NeighborMessage::try_from(&buf[0..size]) else { continue };
        if msg.from == self.id {
            return;
        }; // short-circuit the messages looped back to myself

        match msg.subtype {
            #[allow(unused_variables)]
            NeighborMessageType::Hello {
                address,
                port,
                priority,
                neighbors,
                dr,
                bdr,
            } => {
                let (_, _, _) = (neighbors, dr, bdr);
                let neighbor = match self.neighbors.get_mut(&msg.from) {
                    None => {
                        println!(
                            "Interface {} received new Hello from: {}",
                            self.int_index, msg.from
                        ); //TODO LOG

                        let n = Neighbor {
                            last_hello: Instant::now(),
                            state: NeighborState::Init,
                            address,
                            port,
                        };
                        self.neighbors.insert(msg.from, n);

                        let neighbor_sockaddr = SocketAddrV6::new(address, port, 0, self.int_index);
                        let neighbor_sockaddr = SocketAddr::V6(neighbor_sockaddr);
                        let _ = self.send_hello(&neighbor_sockaddr).await; // TODO do something with an error
                        self.neighbors.get_mut(&msg.from).unwrap() // Just inserted it so i can unwrap it safely
                    }
                    Some(n) => {
                        n.last_hello = Instant::now();
                        n
                    }
                };
            }
            NeighborMessageType::Shutdown => {
                self.neighbors.remove_entry(&msg.from);
            }
        }
    }

    async fn send_hello(&self, dest: &SocketAddr) -> Result<(), io::Error> {
        let subtype = NeighborMessageType::Hello {
            address: self.context.local_addr,
            port: self.context.local_port,
            priority: 1, // TODO needs to be configurable
            neighbors: Vec::new(),
            dr: None,
            bdr: None,
        };
        let hello_msg = NeighborMessage {
            from: self.id,
            version: 1,
            subtype,
        };

        let mut writer = Cursor::new(Vec::new());
        hello_msg.write(&mut writer).unwrap();
        self.context.socket.send_to(writer.get_ref(), dest).await?;
        Ok(())
    }
}

fn new_socket(addr: &SocketAddr) -> io::Result<UdpSocket> {
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&socket2::SockAddr::from(*addr)).unwrap();
    // socket.set_multicast_if_v6(11).unwrap();
    socket.set_nonblocking(true).unwrap();

    let socket = UdpSocket::from_std(socket.into())?;

    Ok(socket)
}

fn join_multicast(addr: SocketAddr) -> io::Result<UdpSocket> {
    let ip_addr = addr.ip();
    let socket = new_socket(&addr).unwrap();

    // depending on the IP protocol we have slightly different work
    match ip_addr {
        IpAddr::V4(_) => {
            panic!("Shouldn't be Ipv4 hello listener");
        }
        IpAddr::V6(v6_addr) => {
            // join to the multicast address, with all interfaces (ipv6 uses indexes not addresses)
            socket.set_multicast_loop_v6(true).unwrap();

            socket.join_multicast_v6(&v6_addr, 11).unwrap();
        }
    };
    Ok(socket)
}

#[allow(dead_code)]
enum NeighborState {
    Down,
    Init,
    TwoWay,
    ExStart,
    Exchange,
    Full,
    Loading,
}

#[allow(dead_code)]
enum NeighborEvent {
    HelloReceived,
    TwoWayReceived,
    NegotiationDone,
    ExchangeDone,
    LoadingDone,
    AdjOk,
    InactivityTimer,
    OneWayReceived,
}

#[allow(dead_code)]
struct Neighbor {
    last_hello: Instant,
    state: NeighborState,
    address: Ipv6Addr,
    port: u16,
}

impl Neighbor {
    fn handle_event(&mut self, event: NeighborEvent, _context: &NDContext) {
        match event {
            NeighborEvent::HelloReceived => self.last_hello = Instant::now(),
            NeighborEvent::TwoWayReceived => todo!(),
            NeighborEvent::NegotiationDone => todo!(),
            NeighborEvent::ExchangeDone => todo!(),
            NeighborEvent::LoadingDone => todo!(),
            NeighborEvent::AdjOk => todo!(),
            NeighborEvent::InactivityTimer => todo!(),
            NeighborEvent::OneWayReceived => todo!(),
        }
    }
}


#[allow(dead_code)]
#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum NDState {
    #[default]
    Down,
    Idle,
    HelloSent,
    Adjacency,
    BDR,
    DR,
}

#[allow(dead_code)]
enum NDEvents {
    Start,
    HelloReceived,
}
#[allow(dead_code)]
pub(crate) enum RouterMsg {}

#[binrw]
#[brw(big)]
#[derive(Clone)]
#[brw(magic = 109_u8)]
struct NeighborMessage {
    // common to all packets
    #[br(parse_with = uuid_parser)]
    #[bw(map = |u: &Uuid|  u.as_bytes())]
    from: Uuid,
    version: u8,
    subtype: NeighborMessageType,
}

#[binrw::parser(reader, endian)]
fn uuid_parser() -> BinResult<Uuid> {
    Ok(Uuid::from_bytes(<_>::read_options(reader, endian, ())?))
}

#[binrw]
#[derive(Clone)]
#[brw(big)]
enum NeighborMessageType {
    #[brw(magic = 1u8)]
    Hello {
        // type 1
        #[bw(map = |a| a.octets())]
        #[br(map = |octets: [u8; 16] | Ipv6Addr::from(octets))]
        address: Ipv6Addr,
        port: u16,
        priority: u8,
        #[br(temp)]
        #[bw(calc = if dr.is_none() {0} else {1} )]
        has_dr: u8,
        #[br(temp)]
        #[bw(calc = if bdr.is_none() {0} else {1})]
        has_bdr: u8,
        #[brw(if(has_dr == 1))]
        #[br(map = |b: [u8; 16]| Some(Uuid::from_bytes(b)))]
        #[bw(map = |u| { *u.unwrap().as_bytes() })]
        dr: Option<Uuid>,
        #[brw(if(has_bdr == 1))]
        #[br(map = |b: [u8; 16]| Some(Uuid::from_bytes(b)))]
        #[bw(map = |u| { *u.unwrap().as_bytes() })]
        bdr: Option<Uuid>,
        #[br(temp)]
        #[bw(try_calc(u16::try_from(neighbors.len())))]
        len: u16,
        #[br(parse_with = count_with(len as usize, uuid_parser))]
        // #[br(dbg)]
        #[bw(map = |v: &Vec::<Uuid>|  v.iter().fold(Vec::<u8>::new(), |mut v,u| {v.extend(u.as_bytes()); v}))]
        neighbors: Vec<Uuid>,
    },
    #[brw(magic = 2u8)]
    Shutdown,
}
