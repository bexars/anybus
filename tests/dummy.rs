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

/// Raising an anycast listener's cost moves the next message to the other bus.
#[tokio::test]
async fn dummy_link_anycast_set_cost() {
    let mut hub = AnyBus::new();
    let mut left = AnyBus::new();
    let mut right = AnyBus::new();
    hub.run();
    left.run();
    right.run();

    let (hub_left, left_end) = duplex(8 * 1024);
    let (hub_right, right_end) = duplex(8 * 1024);
    let _hub_left_link = hub.new_dummy_peer(hub_left);
    let _left_link = left.new_dummy_peer(left_end);
    let _hub_right_link = hub.new_dummy_peer(hub_right);
    let _right_link = right.new_dummy_peer(right_end);

    let mut left_listener = left
        .handle()
        .clone()
        .listener()
        .cost(1)
        .anycast()
        .register::<common::NumberMessage>()
        .await
        .unwrap();
    let mut right_listener = right
        .handle()
        .clone()
        .listener()
        .cost(10)
        .anycast()
        .register::<common::NumberMessage>()
        .await
        .unwrap();

    let hub_handle = hub.handle().clone();
    let first = timeout(Duration::from_secs(3), async {
        loop {
            if hub_handle.send(common::NumberMessage { value: 1 }).is_err() {
                sleep(Duration::from_millis(20)).await;
                continue;
            }
            tokio::select! {
                message = left_listener.recv() => {
                    assert_eq!(message.unwrap().value, 1);
                    return "left";
                }
                message = right_listener.recv() => {
                    assert_eq!(message.unwrap().value, 1);
                    return "right";
                }
                _ = sleep(Duration::from_millis(50)) => {}
            }
        }
    })
    .await
    .expect("first anycast delivery");
    assert_eq!(first, "left");

    left_listener.set_cost(100).await.unwrap();

    let second = timeout(Duration::from_secs(3), async {
        loop {
            if hub_handle.send(common::NumberMessage { value: 2 }).is_err() {
                sleep(Duration::from_millis(20)).await;
                continue;
            }
            tokio::select! {
                message = left_listener.recv() => {
                    let _ = message.unwrap();
                }
                message = right_listener.recv() => {
                    assert_eq!(message.unwrap().value, 2);
                    return "right";
                }
                _ = sleep(Duration::from_millis(50)) => {}
            }
        }
    })
    .await
    .expect("anycast delivery after set_cost");
    assert_eq!(second, "right");

    hub.shutdown(None);
    left.shutdown(None);
    right.shutdown(None);
}
