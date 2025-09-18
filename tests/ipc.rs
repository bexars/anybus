mod common;
use crate::common::NumberMessage;
// use tracing::info;

#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_unicast_two_buses() {
    // tracing_subscriber::fmt::init();

    use tokio::time;

    let mb1 = anybus::AnyBus::new();
    let mb2 = anybus::AnyBus::new();
    let mut handle1 = mb1.handle().clone();
    let handle2 = mb2.handle().clone();
    let mut listener1 = handle1.register_unicast::<NumberMessage>().await.unwrap();
    time::sleep(std::time::Duration::from_secs(1)).await;
    handle2.send(NumberMessage { value: 100 }).unwrap();
    let msg: NumberMessage = listener1.recv().await.unwrap();
    assert_eq!(msg.value, 100);
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_unicast_three_buses_with_drop() {
    // tracing_subscriber::fmt::init();

    use tokio::time;

    let mb1 = anybus::AnyBus::new();
    let mb2 = anybus::AnyBus::new();
    let mut handle1 = mb1.handle().clone();
    let handle2 = mb2.handle().clone();
    let mut listener1 = handle1.register_unicast::<NumberMessage>().await.unwrap();
    time::sleep(std::time::Duration::from_millis(10)).await;
    handle2.send(NumberMessage { value: 100 }).unwrap();

    let msg: NumberMessage = listener1.recv().await.unwrap();
    assert_eq!(msg.value, 100);
    drop(mb1);
    drop(handle1);
    drop(listener1);
    time::sleep(std::time::Duration::from_millis(10)).await;

    let mb3 = anybus::AnyBus::new();
    let mut handle3 = mb3.handle().clone();
    let mut listener3 = handle3.register_unicast::<NumberMessage>().await.unwrap();
    time::sleep(std::time::Duration::from_millis(10)).await;

    handle2.send(NumberMessage { value: 200 }).unwrap();
    let msg = listener3.recv().await.unwrap();
    assert_eq!(msg.value, 200);
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_rpc_two_busses() {
    use tokio::task::yield_now;

    // tracing_subscriber::fmt::init();
    let mb1 = anybus::AnyBus::new();
    let mb2 = anybus::AnyBus::new();
    let mut handle1 = mb1.handle().clone();
    let handle2 = mb2.handle().clone();
    let mut listener1 = handle1.register_rpc::<common::RpcMessage>().await.unwrap();
    tokio::spawn(async move {
        let mut responder = listener1.recv().await.unwrap();
        let msg = responder.payload().unwrap();
        assert_eq!(msg.value, 41);
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

        responder.reply(common::RpcResponse { value: 123 }).unwrap();
    });
    yield_now().await;
    yield_now().await;
    yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let response = handle2
        .rpc_once(common::RpcMessage { value: 41 })
        .await
        .unwrap();
    assert_eq!(response.value, 123);
}

/// This test verifies that anycast messages are delivered to local listeners first,
/// and if no local listeners are available, they are sent to remote listeners after the local ones are dropped.
#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_anycast_two_busses_local_then_remote() {
    use tokio::time;

    // tracing_subscriber::fmt::init();
    let builder = anybus::AnyBusBuilder::new().enable_ipc(true);
    let mut mb1 = builder.build();
    let mut mb2 = builder.build();
    mb1.run();
    mb2.run();
    let mut handle1 = mb1.handle().clone();
    let mut handle2 = mb2.handle().clone();
    let mut listener1 = handle1
        .register_anycast::<common::StringMessage>()
        .await
        .unwrap();

    let mut listener2 = handle2
        .register_anycast::<common::StringMessage>()
        .await
        .unwrap();
    handle2
        .send(common::StringMessage {
            value: "Hello, World!".into(),
        })
        .unwrap();
    let msg: common::StringMessage = listener2.recv().await.unwrap();
    assert_eq!(msg.value, "Hello, World!");
    // info!("Dropping listener2");
    drop(listener2);
    // yield_now().await;
    // yield_now().await;
    time::sleep(std::time::Duration::from_millis(10)).await;
    handle2
        .send(common::StringMessage {
            value: "Hello, World!".into(),
        })
        .unwrap();
    let msg: common::StringMessage = listener1.recv().await.unwrap();
    assert_eq!(msg.value, "Hello, World!");
}

// #[cfg(feature = "tokio")]
// #[tokio::test]
// async fn test_multicast_several_busses() {
//     // tracing_subscriber::fmt::init();

//     let mb1 = anybus::AnyBus::new();
//     let mb2 = anybus::AnyBus::new();
//     let mb3 = anybus::AnyBus::new();
//     let mut handle1 = mb1.handle().clone();
//     let mut handle2 = mb2.handle().clone();
//     let handle3 = mb3.handle().clone();
//     let mut listener1 = handle1.register_multicast::<NumberMessage>().await.unwrap();
//     let mut listener2 = handle2.register_multicast::<NumberMessage>().await.unwrap();
//     tokio::time::sleep(std::time::Duration::from_millis(100)).await;
//     handle1.send(NumberMessage { value: 42 }).unwrap();
//     tokio::time::sleep(std::time::Duration::from_millis(100)).await;

//     handle2.send(NumberMessage { value: 100 }).unwrap();
//     tokio::time::sleep(std::time::Duration::from_millis(100)).await;

//     handle3.send(NumberMessage { value: 7 }).unwrap();

//     let msg: NumberMessage = listener1.recv().await.unwrap();
//     assert_eq!(msg.value, 42);
//     let msg: NumberMessage = listener2.recv().await.unwrap();
//     assert_eq!(msg.value, 42);
//     let msg: NumberMessage = listener1.recv().await.unwrap();
//     assert_eq!(msg.value, 100);

//     let msg: NumberMessage = listener2.recv().await.unwrap();
//     assert_eq!(msg.value, 100);
//     let msg: NumberMessage = listener1.recv().await.unwrap();
//     assert_eq!(msg.value, 7);
//     let msg: NumberMessage = listener2.recv().await.unwrap();
//     assert_eq!(msg.value, 7);
// }

// #[cfg(feature = "tokio")]
// #[tokio::test]
// async fn test_multicast_several_bus_drop() {
//     use tracing::info;

//     // tracing_subscriber::fmt::init();

//     let mut mb1 = anybus::AnyBus::new();
//     tokio::time::sleep(std::time::Duration::from_millis(10)).await;

//     let mb2 = anybus::AnyBus::new();
//     tokio::time::sleep(std::time::Duration::from_millis(10)).await;

//     let mb3 = anybus::AnyBus::new();

//     let mut handle1 = mb1.handle().clone();
//     let mut handle2 = mb2.handle().clone();
//     let _handle3 = mb3.handle().clone();
//     let listener1 = handle1.register_multicast::<NumberMessage>().await.unwrap();
//     let mut listener2 = handle2.register_multicast::<NumberMessage>().await.unwrap();
//     // let mut Listener3 = handle3.register_multicast::<NumberMessage>().await.unwrap();
//     tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
//     handle2.send(NumberMessage { value: 42 }).unwrap();
//     tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
//     tokio::task::yield_now().await;
//     tokio::task::yield_now().await;
//     mb1.shutdown();
//     // drop(handle1);
//     drop(listener1);
//     tokio::time::sleep(std::time::Duration::from_millis(100)).await;
//     handle2.send(NumberMessage { value: 100 }).unwrap();
//     let msg = listener2.recv().await.unwrap();
//     assert_eq!(msg.value, 42);
//     info!("Passed the first assert");
//     let msg = listener2.recv().await.unwrap();
//     assert_eq!(msg.value, 100);
//     drop(mb1);
//     drop(mb2);
//     drop(mb3);
// }
