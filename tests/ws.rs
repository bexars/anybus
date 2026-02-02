use anybus::Handle;
use std::time::Duration;
use tokio::time::sleep;

#[cfg(feature = "ws")]
use url::Url;
// use url::Url;

#[cfg(all(feature = "ws_server", feature = "tokio"))]
#[tokio::test]
async fn test_ws_connection() {
    use std::net::Ipv4Addr;

    use anybus::AnyBusBuilder;

    let bus = AnyBusBuilder::new().ws_listener(anybus::peers::WsListenerOptions {
        addr: Ipv4Addr::LOCALHOST.into(),
        port: 10800,
        use_tls: true,
        cert_path: Some("./server.crt".into()),
        key_path: Some("./server.key".into()),
    });
    let mut bus = bus.init();
    bus.run();
    let _handle: Handle = bus.handle().clone();

    // tokio::spawn(async move {
    //     bus.run();
    // });

    sleep(Duration::from_millis(100)).await; // Give the bus time to start

    let _url = Url::parse("ws://localhost:10800").unwrap();
}
