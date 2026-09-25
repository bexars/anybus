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

/// A hub and two leaves. Anycast follows the cheaper leaf, then the other one
/// after that leaf's low-cost listener is dropped and its advertisement rises.
#[tokio::test]
async fn dummy_link_anycast_follows_lower_cost() {
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

    let left_handle = left.handle().clone();
    let mut left_low = left_handle
        .listener()
        .cost(1)
        .anycast()
        .register::<common::NumberMessage>()
        .await
        .unwrap();
    let mut left_high = left_handle
        .listener()
        .cost(100)
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
                message = left_low.recv() => {
                    assert_eq!(message.unwrap().value, 1);
                    return "left-low";
                }
                message = left_high.recv() => {
                    assert_eq!(message.unwrap().value, 1);
                    return "left-high";
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
    assert_eq!(first, "left-low");

    drop(left_low);

    let second = timeout(Duration::from_secs(3), async {
        loop {
            if hub_handle.send(common::NumberMessage { value: 2 }).is_err() {
                sleep(Duration::from_millis(20)).await;
                continue;
            }
            tokio::select! {
                message = left_high.recv() => {
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
    .expect("anycast delivery after the low-cost listener is dropped");
    assert_eq!(second, "right");

    hub.shutdown(None);
    left.shutdown(None);
    right.shutdown(None);
}

/// Three buses in a triangle. Anycast uses the cheap link, then the other bus
/// after that link shuts down and the remaining path costs more.
#[tokio::test]
async fn dummy_link_anycast_fails_over_when_cheap_link_drops() {
    let mut hub = AnyBus::new();
    let mut near = AnyBus::new();
    let mut far = AnyBus::new();
    hub.run();
    near.run();
    far.run();

    let (hub_near_end, near_hub_end) = duplex(8 * 1024);
    let (hub_far_end, far_hub_end) = duplex(8 * 1024);
    let (near_far_end, far_near_end) = duplex(8 * 1024);
    let cheap_link = hub.new_dummy_peer_with(hub_near_end, 1, &[]);
    let _near_hub = near.new_dummy_peer_with(near_hub_end, 1, &[]);
    let _hub_far = hub.new_dummy_peer_with(hub_far_end, 10, &[]);
    let _far_hub = far.new_dummy_peer_with(far_hub_end, 10, &[]);
    let _near_far = near.new_dummy_peer_with(near_far_end, 40, &[]);
    let _far_near = far.new_dummy_peer_with(far_near_end, 40, &[]);

    let mut near_listener = near
        .handle()
        .clone()
        .register_anycast::<common::NumberMessage>()
        .await
        .unwrap();
    let mut far_listener = far
        .handle()
        .clone()
        .register_anycast::<common::NumberMessage>()
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
                message = near_listener.recv() => {
                    assert_eq!(message.unwrap().value, 1);
                    return "near";
                }
                message = far_listener.recv() => {
                    assert_eq!(message.unwrap().value, 1);
                    return "far";
                }
                _ = sleep(Duration::from_millis(50)) => {}
            }
        }
    })
    .await
    .expect("first anycast delivery");
    assert_eq!(first, "near");

    cheap_link.shutdown().unwrap();

    let second = timeout(Duration::from_secs(3), async {
        loop {
            if hub_handle.send(common::NumberMessage { value: 2 }).is_err() {
                sleep(Duration::from_millis(20)).await;
                continue;
            }
            tokio::select! {
                message = near_listener.recv() => {
                    let _ = message.unwrap();
                }
                message = far_listener.recv() => {
                    assert_eq!(message.unwrap().value, 2);
                    return "far";
                }
                _ = sleep(Duration::from_millis(50)) => {}
            }
        }
    })
    .await
    .expect("anycast delivery after the cheap link shuts down");
    assert_eq!(second, "far");

    hub.shutdown(None);
    near.shutdown(None);
    far.shutdown(None);
}
