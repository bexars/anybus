use lazy_static::*;
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;

use std::io::{self, Cursor};
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
const MAGIC: u8 = 109; // 'm' for Msgbus
pub(crate) struct NeighborDiscovery {
    id: Uuid,
    state: NDState,
    bcs_rx: tokio::sync::watch::Receiver<BusControlMsg>,
}

lazy_static! {
    // pub static ref IPV4: IpAddr = Ipv4Addr::new(224, 0, 0, 123).into();
    pub static ref LISTEN_ADDRESS: Ipv6Addr = Ipv6Addr::new(0xFF02, 0, 0, 0, 0, 0, 0, 0x0123).into();
}
const LISTEN_PORT: u16 = 3069;

impl NeighborDiscovery {
    pub(crate) fn new(id: Uuid, bcs_rx: tokio::sync::watch::Receiver<BusControlMsg>) -> Self {
        let state = {
            let state: tokio::sync::watch::Ref<'_, BusControlMsg> = bcs_rx.borrow();

            match *state {
                BusControlMsg::Run => NDState::Idle,
                BusControlMsg::Shutdown => NDState::Shutdown,
            }
        };
        Self { id, state, bcs_rx }
    }

    pub(crate) async fn run(mut self) {
        self.state = NDState::Idle;
        let addr = SocketAddr::new(IpAddr::V6(*LISTEN_ADDRESS), LISTEN_PORT);
        let mult_socket = join_multicast(addr).unwrap();

        let mut buf = vec![0u8; 9000];

        let socket = tokio::net::UdpSocket::bind("[::]:0").await.unwrap();
        socket.join_multicast_v6(&*LISTEN_ADDRESS, 11).unwrap();

        let sock_addr = socket.local_addr().unwrap();
        let IpAddr::V6(ip_addr) = sock_addr.ip() else { panic!() };

        let mut neighbor_map = HashMap::new();

        let mut hello_timer = time::interval(Duration::from_secs(10));
        hello_timer.set_missed_tick_behavior(MissedTickBehavior::Delay); // 10 second
                                                                         // let mut  buf: Vec<u8> = Vec::new();

        loop {
            select! {
                _ = self.bcs_rx.changed() => {
                    let state = self.bcs_rx.borrow();
                    match *state {
                        BusControlMsg::Run => { continue },
                        BusControlMsg::Shutdown => {
                            self.state = NDState::Shutdown;
                            break;
                        },
                    }

                },
                Ok(size) = mult_socket.recv(&mut buf) => {

                    let mut cursor = Cursor::new(&mut buf[0..size]);
                    let msg = match NeighborMessage::read(&mut cursor) {
                        Ok(msg) => msg,
                        Err(e) => {
                            eprintln!("Neighbor message decode failure {:?}",e);  //TODO LOG
                            continue} ,
                    };
                    // let Ok(msg) = NeighborMessage::try_from(&buf[0..size]) else { continue };
                    if msg.from == self.id { continue };  // short-circuit the messages looped back to myself
                    match msg.subtype {
                        NeighborMessageType::Hello { address, neighbors, dr, bdr } => {
                            let (_,_,_ ) = (neighbors, dr, bdr);
                            match neighbor_map.get_mut(&msg.from) {
                                None => {
                                    println!("Router {} received new Hello from: {}", self.id, msg.from); //TODO LOG

                                    let n = Neighbor {
                                        last_hello: Instant::now(),
                                        state: NeighborState::Normal,
                                        address
                                    };
                                neighbor_map.insert(msg.from, n);
                            },
                                Some(n) => {
                                    n.last_hello = Instant::now();
                                }


                        };
                        },
                        NeighborMessageType::Shutdown => {
                            neighbor_map.remove_entry(&msg.from);
                        }
                    }
                },
                now = hello_timer.tick() => {
                        let subtype = NeighborMessageType::Hello{
                            address: ip_addr,
                            neighbors: Vec::new(),
                            dr: None,
                            bdr: None };
                        let hello_msg = NeighborMessage { from: self.id, version: 1, subtype };

                        // hello_msg.to_bytes(&mut buf);
                        let mut writer = Cursor::new(Vec::new());

                        hello_msg.write(&mut writer).unwrap();
                        let addr = SocketAddr::from_str("[ff02::123%11]:3069").unwrap();


                        socket.send_to(writer.get_ref(), addr).await.unwrap();
                        let mut dead_list = Vec::new();
                        for (id, n) in neighbor_map.iter() {
                            let diff = now - n.last_hello;
                            println!("{:?} ### {id}", diff);
                            if diff > Duration::from_secs(40) {
                                println!("Dead timer expired for {id}"); //TODO Make this a log output
                                dead_list.push(*id);
                            };
                        };
                        for id in dead_list {
                            neighbor_map.remove(&id);
                        }

                }
            }
        }
        if self.state == NDState::Shutdown {
            let shutdown_msg = NeighborMessage {
                from: self.id,
                version: VERSION,
                subtype: NeighborMessageType::Shutdown,
            };
            let mut writer = Cursor::new(Vec::new());

            shutdown_msg.write(&mut writer).unwrap();
            let _ = socket.send(writer.get_ref()).await;
        }
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

enum NeighborState {
    Normal,
    DR,
    BDR,
}

struct Neighbor {
    last_hello: Instant,
    state: NeighborState,
    address: Ipv6Addr,
}

#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum NDState {
    #[default]
    Shutdown,
    Idle,
    Hello,
    Adjacency,
}

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

// impl NeighborMessage {
//     fn to_bytes(&self, buf: &mut Vec<u8>) {
//         buf.clear();
//         buf.push(MAGIC); // u8
//         buf.push(VERSION); // u8
//         buf.extend(self.from.as_bytes()); // 16 bytes
//         self.subtype.to_bytes(buf);
//     }
// }

// impl TryFrom<&[u8]> for NeighborMessage {
//     type Error = Box<dyn Error>;

//     fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
//         let mut bytes = value.iter();
//         let _magic: &u8 = match bytes.next() {
//             Some(m) if *m == MAGIC => m,
//             Some(_) => return Err("Bad magic".into()),
//             None => return Err("Unexpected EOF".into()),
//         };
//         let version = match bytes.next() {
//             Some(m) if *m == VERSION => *m,
//             Some(_) => return Err("Bad version".into()),
//             None => return Err("Unexpected EOF".into()),
//         };
//         let Ok(from) = Uuid::from_slice(&bytes.as_slice()[0..16]) else { return Err("Unexpected EOF".into())};
//         for _ in 0..16 {
//             bytes.next();
//         }

//         let subtype = NeighborMessageType::try_from(bytes.as_slice())?;

//         Ok(NeighborMessage {
//             from,
//             version,
//             subtype,
//         })

//         // Err("Placeholder".into())
//     }
// }

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
        // #[br(count = len)]
        // #[br(args { count: len.into() })]
        // #[br(parse_with = vec_uuid_parser)]
        #[br(parse_with = count_with(len as usize, uuid_parser))]
        // #[br(dbg)]
        #[bw(map = |v: &Vec::<Uuid>|  v.iter().fold(Vec::<u8>::new(), |mut v,u| {v.extend(u.as_bytes()); v}))]
        // #[br(parse_with = count_with(len as usize, |reader,endian,a|uuid_parser(reader, endian, a)))]
        // #[br(map = |vb: Vec::<[u8; 16]> |   { vb.iter().map(|bytes| Uuid::from_bytes(*bytes)).collect::<Vec::<Uuid>>()})]
        neighbors: Vec<Uuid>,
    },
    #[brw(magic = 2u8)]
    Shutdown,
}

// impl NeighborMessageType {
//     fn to_bytes(&self, buf: &mut Vec<u8>) {
//         // buf already has the common header so don't clear it
//         match self {
//             NeighborMessageType::Hello {
//                 address,
//                 dr,
//                 bdr,
//                 neighbors,
//             } => {
//                 buf.push(1); // this type (Hello)
//                 buf.extend(address.octets()); // 16 octets
//                 buf.push(if dr.is_none() { 0 } else { 1 });
//                 buf.push(if bdr.is_none() { 0 } else { 1 });
//                 dr.iter().for_each(|u| buf.extend(u.as_bytes()));
//                 bdr.iter().for_each(|u| buf.extend(u.as_bytes()));
//                 buf.extend((neighbors.len() as u16).to_be_bytes()); // 2 octets
//                 neighbors.iter().for_each(|u| buf.extend(u.as_bytes()));
//             }
//         }
//     }
// }

// impl TryFrom<&[u8]> for NeighborMessageType {
//     type Error = Box<dyn Error>;

//     fn try_from(buf: &[u8]) -> Result<Self, Self::Error> {
//         let mut cursor: usize = 0;
//         let subtype: u8 = buf[cursor];
//         cursor += 1;

//         match subtype {
//             1 => {
//                 let address: [u8; 16] = buf[0..16].try_into()?;
//                 cursor += 16;
//                 let address = Ipv6Addr::from(address);
//                 let is_dr = buf[16] == 1;
//                 let is_bdr = buf[17] == 1;
//                 cursor += 2;
//                 let dr = if is_dr {
//                     let a = Uuid::from_slice(&buf[cursor..cursor + 16])?;
//                     cursor += 16;
//                     Some(a)
//                 } else {
//                     None
//                 };
//                 let bdr = if is_dr {
//                     let a = Uuid::from_slice(&buf[cursor..cursor + 16])?;
//                     cursor += 16;
//                     Some(a)
//                 } else {
//                     None
//                 };

//                 let veclen = (buf[cursor] as u16) << 8 + buf[cursor + 1] as u16;
//                 cursor += 2;
//                 if buf.len() - cursor < veclen as usize * 16 {
//                     return Err("Unexpected eOF".into());
//                 }
//                 let mut neighbors = Vec::new();
//                 for _ in 0..veclen {
//                     let u = Uuid::from_slice(&buf[cursor..cursor + 16])?;
//                     neighbors.push(u);
//                 }
//                 Ok(NeighborMessageType::Hello {
//                     address,
//                     neighbors,
//                     dr,
//                     bdr,
//                 })
//             }
//             _ => return Err("Bad packet type".into()),
//         }
//     }
// }
