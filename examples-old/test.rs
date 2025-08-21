use std::{
    net::{IpAddr, SocketAddr, UdpSocket},
    str::FromStr,
    time::Duration,
};
use tokio::select;

// lazy_static! {
//     // pub static ref IPV4: IpAddr = Ipv4Addr::new(224, 0, 0, 123).into();
//     pub static ref LISTEN_ADDRESS: IpAddr = Ipv6Addr::new(0xFF02, 0, 0, 0, 0, 0, 0, 0x0001).into();
// }
// const LISTEN_PORT: u16 = 3069;

#[tokio::main]
async fn main() {
    // let addr = SocketAddr::new(*LISTEN_ADDRESS, LISTEN_PORT);
    let sock_addr = SocketAddr::from_str("[ff02::123%11]:3069").unwrap();
    let IpAddr::V6(ip_addr) = sock_addr.ip() else {
        panic!()
    };

    let socket = tokio::net::UdpSocket::bind(sock_addr).await.unwrap();
    socket.join_multicast_v6(&ip_addr, 11).unwrap();

    let out_socket = UdpSocket::bind("[::%11]:0").unwrap();

    let mut timer = tokio::time::interval(Duration::from_secs(5));
    let mut buf = vec![0; 2000];
    loop {
        select! {
            Ok(msg) = socket.recv(&mut buf) => {

                dbg!(msg, &buf[0..msg]);
                // buf.clear();
            },
            _ = timer.tick() => {
                println!("Tick");
                out_socket.send_to(b"Hello world", sock_addr).unwrap();
                println!("Tock");
            }
        }
    }
}
