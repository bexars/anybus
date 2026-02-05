use std::{io::stdin, time::Duration};

use anybus::Handle;
use tokio::time::sleep;
#[cfg(feature = "ws")]
use url::Url;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::fmt()
        .with_max_level(tracing_subscriber::filter::LevelFilter::TRACE)
        .init();

    use std::net::Ipv4Addr;

    use anybus::AnyBusBuilder;

    let bus = AnyBusBuilder::new()
        .ws_listener(anybus::peers::WsListenerOptions {
            addr: Ipv4Addr::from_octets([0, 0, 0, 0]).into(),
            port: 10800,
            use_tls: true,
            cert_path: Some("./server.crt".into()),
            key_path: Some("./server.key".into()),
        })
        .enable_ipc(true);
    let mut bus = bus.init();
    bus.run();
    let _handle: Handle = bus.handle().clone();

    sleep(Duration::from_millis(100)).await; // Give the bus time to start

    let _url = Url::parse("ws://localhost:10800").unwrap();
    stdin().read_line(&mut String::new()).unwrap();
}
