#![cfg(feature = "dummy_peer")]

mod common;

use std::time::Duration;

use anybus::AnyBus;
use anybus::tokio;
use tokio::io::duplex;
use tokio::spawn;
use tokio::time::{sleep, timeout};

/// Two buses connected by a duplex pair complete one RPC.
#[tokio::test]
async fn dummy_link_rpc_round_trip() {
    let mut server = AnyBus::new();
    let mut client = AnyBus::new();
    server.run();
    client.run();

    let (server_end, client_end) = duplex(8 * 1024);
    let _server_link = server.new_dummy_peer(server_end);
    let _client_link = client.new_dummy_peer(client_end);

    let server_handle = server.handle().clone();
    let mut listener = server_handle
        .register_rpc::<common::RpcMessage>()
        .await
        .unwrap();
    let responder = spawn(async move {
        let mut request = listener.recv().await.unwrap();
        let message = request.payload().unwrap();
        request
            .reply(common::RpcResponse {
                value: i32::from(message.value) * 20,
            })
            .unwrap();
    });

    let client_handle = client.handle().clone();
    let response = timeout(Duration::from_secs(3), async move {
        loop {
            match client_handle
                .rpc_once(common::RpcMessage { value: 5 })
                .await
            {
                Ok(response) => return response,
                Err(_) => sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .expect("rpc over the dummy link");

    assert_eq!(response.value, 100);
    responder.await.unwrap();
    server.shutdown(None);
    client.shutdown(None);
}
