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

fn answer(mut request: anybus::RpcRequest<common::RpcMessage>, factor: i32) -> i32 {
    let value = i32::from(request.payload().unwrap().value) * factor;
    request.reply(common::RpcResponse { value }).unwrap();
    value
}

/// Two anycast RPC listeners on one bus. The cheaper one receives the request,
/// then raising its cost moves the next request to the other listener.
#[tokio::test]
async fn anycast_rpc_follows_set_cost() {
    let mut bus = AnyBus::new();
    bus.run();
    let handle = bus.handle().clone();

    let mut cheap = handle
        .listener()
        .cost(1)
        .anycast()
        .rpc()
        .register::<common::RpcMessage>()
        .await
        .unwrap();
    let mut dear = handle
        .listener()
        .cost(10)
        .anycast()
        .rpc()
        .register::<common::RpcMessage>()
        .await
        .unwrap();

    let caller = handle.clone();
    let first_call = spawn(async move { caller.rpc_once(common::RpcMessage { value: 3 }).await });
    let first = timeout(Duration::from_secs(3), async {
        tokio::select! {
            request = cheap.recv() => {
                assert_eq!(answer(request.unwrap(), 2), 6);
                "cheap"
            }
            request = dear.recv() => {
                let _ = answer(request.unwrap(), 2);
                "dear"
            }
        }
    })
    .await
    .expect("first anycast rpc");
    assert_eq!(first, "cheap");
    assert_eq!(first_call.await.unwrap().unwrap().value, 6);

    cheap.set_cost(100).await.unwrap();

    let second = timeout(Duration::from_secs(3), async {
        loop {
            let caller = handle.clone();
            let call = spawn(async move { caller.rpc_once(common::RpcMessage { value: 4 }).await });
            tokio::select! {
                request = cheap.recv() => {
                    let _ = answer(request.unwrap(), 2);
                    let _ = call.await;
                }
                request = dear.recv() => {
                    assert_eq!(answer(request.unwrap(), 2), 8);
                    assert_eq!(call.await.unwrap().unwrap().value, 8);
                    return "dear";
                }
            }
        }
    })
    .await
    .expect("anycast rpc after set_cost");
    assert_eq!(second, "dear");

    cheap.set_cost(1).await.unwrap();

    let third = timeout(Duration::from_secs(3), async {
        loop {
            let caller = handle.clone();
            let call = spawn(async move { caller.rpc_once(common::RpcMessage { value: 5 }).await });
            tokio::select! {
                request = cheap.recv() => {
                    assert_eq!(answer(request.unwrap(), 2), 10);
                    assert_eq!(call.await.unwrap().unwrap().value, 10);
                    return "cheap";
                }
                request = dear.recv() => {
                    let _ = answer(request.unwrap(), 2);
                    let _ = call.await;
                }
            }
        }
    })
    .await
    .expect("anycast rpc after lowering the cost");
    assert_eq!(third, "cheap");

    let duplicate = handle.register_rpc::<common::RpcMessage>().await;
    assert!(duplicate.is_err());

    bus.shutdown(None);
}

/// An anycast RPC crosses a dummy link and follows the cheaper leaf.
#[tokio::test]
async fn dummy_link_anycast_rpc_follows_set_cost() {
    let mut hub = AnyBus::new();
    let mut left = AnyBus::new();
    let mut right = AnyBus::new();
    hub.run();
    left.run();
    right.run();

    let (hub_left, left_end) = duplex(8 * 1024);
    let (hub_right, right_end) = duplex(8 * 1024);
    let _hub_left = hub.new_dummy_peer(hub_left);
    let _left = left.new_dummy_peer(left_end);
    let _hub_right = hub.new_dummy_peer(hub_right);
    let _right = right.new_dummy_peer(right_end);

    let mut left_rpc = left
        .handle()
        .clone()
        .listener()
        .cost(1)
        .anycast()
        .rpc()
        .register::<common::RpcMessage>()
        .await
        .unwrap();
    let mut right_rpc = right
        .handle()
        .clone()
        .listener()
        .cost(10)
        .anycast()
        .rpc()
        .register::<common::RpcMessage>()
        .await
        .unwrap();

    let hub_handle = hub.handle().clone();
    let first = timeout(Duration::from_secs(3), async {
        loop {
            let caller = hub_handle.clone();
            let call = spawn(async move { caller.rpc_once(common::RpcMessage { value: 5 }).await });
            tokio::select! {
                request = left_rpc.recv() => {
                    assert_eq!(answer(request.unwrap(), 20), 100);
                    assert_eq!(call.await.unwrap().unwrap().value, 100);
                    return "left";
                }
                request = right_rpc.recv() => {
                    let _ = answer(request.unwrap(), 20);
                    let _ = call.await;
                    return "right";
                }
                _ = sleep(Duration::from_millis(20)) => {
                    call.abort();
                }
            }
        }
    })
    .await
    .expect("first rpc over the dummy link");
    assert_eq!(first, "left");

    left_rpc.set_cost(100).await.unwrap();

    let second = timeout(Duration::from_secs(3), async {
        loop {
            let caller = hub_handle.clone();
            let call = spawn(async move { caller.rpc_once(common::RpcMessage { value: 6 }).await });
            tokio::select! {
                request = left_rpc.recv() => {
                    let _ = answer(request.unwrap(), 20);
                    let _ = call.await;
                }
                request = right_rpc.recv() => {
                    assert_eq!(answer(request.unwrap(), 20), 120);
                    assert_eq!(call.await.unwrap().unwrap().value, 120);
                    return "right";
                }
                _ = sleep(Duration::from_millis(20)) => {
                    call.abort();
                }
            }
        }
    })
    .await
    .expect("rpc after set_cost");
    assert_eq!(second, "right");

    left_rpc.set_cost(1).await.unwrap();

    let third = timeout(Duration::from_secs(3), async {
        loop {
            let caller = hub_handle.clone();
            let call = spawn(async move { caller.rpc_once(common::RpcMessage { value: 7 }).await });
            tokio::select! {
                request = left_rpc.recv() => {
                    assert_eq!(answer(request.unwrap(), 20), 140);
                    assert_eq!(call.await.unwrap().unwrap().value, 140);
                    return "left";
                }
                request = right_rpc.recv() => {
                    let _ = answer(request.unwrap(), 20);
                    let _ = call.await;
                }
                _ = sleep(Duration::from_millis(20)) => {
                    call.abort();
                }
            }
        }
    })
    .await
    .expect("rpc after lowering the cost");
    assert_eq!(third, "left");

    hub.shutdown(None);
    left.shutdown(None);
    right.shutdown(None);
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

    left_listener.set_cost(1).await.unwrap();

    let third = timeout(Duration::from_secs(3), async {
        loop {
            if hub_handle.send(common::NumberMessage { value: 3 }).is_err() {
                sleep(Duration::from_millis(20)).await;
                continue;
            }
            tokio::select! {
                message = left_listener.recv() => {
                    assert_eq!(message.unwrap().value, 3);
                    return "left";
                }
                message = right_listener.recv() => {
                    let _ = message.unwrap();
                }
                _ = sleep(Duration::from_millis(50)) => {}
            }
        }
    })
    .await
    .expect("anycast delivery after lowering the cost");
    assert_eq!(third, "left");

    hub.shutdown(None);
    left.shutdown(None);
    right.shutdown(None);
}
