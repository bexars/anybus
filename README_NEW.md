# Anybus

A serverless message bus for easy communication between Rust applications over local networks and the internet.

Anybus enables seamless messaging between services using a variety of transport mechanisms including IPC (Inter-Process Communication) for local communication and WebSockets for remote connections. It supports different messaging patterns: unicast, anycast, broadcast, and RPC (Remote Procedure Call).

## Features

- **Multiple Transports**: IPC for local communication, WebSockets for remote.
- **Messaging Patterns**: Unicast, Anycast, Broadcast, and RPC.
- **Type-Safe RPC**: Procedural macro for generating type-safe RPC clients and servers.
- **Managed Services**: Built-in support for managed RPC depots and message stops.
- **Async/Await**: Fully asynchronous using Tokio.
- **WASM Support**: Can run in WebAssembly environments.

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
anybus = "0.2"
```

For full features:

```toml
[dependencies]
anybus = { version = "0.2", features = ["ipc", "ws", "ws_server", "serde"] }
```

## Quick Start

### Basic Setup

```rust
use anybus::{AnyBus, BusRider, BusRiderWithUuid};
use uuid::Uuid;

#[derive(Clone, Debug)]
struct MyMessage {
    content: String,
}

impl BusRider for MyMessage {}
impl BusRiderWithUuid for MyMessage {
    const ANYBUS_UUID: Uuid = uuid::from_u128(0x1234567890abcdef1234567890abcdef);
}

// Create and start Anybus
let mut anybus = AnyBus::build()
    .enable_ipc(true)
    .init();
anybus.run();

// Get a handle for communication
let handle = anybus.handle().clone();
```

### Sending Messages

```rust
// Send a message
handle.send(MyMessage { content: "Hello!".to_string() }).await?;
```

### Receiving Messages

```rust
// Register a unicast receiver
let receiver = handle.listener()
    .unicast::<MyMessage>()
    .register()
    .await?;

// Receive messages
while let Some(msg) = receiver.recv().await {
    println!("Received: {}", msg.content);
}
```

### RPC with Macros

The `#[anybus_rpc]` macro generates RPC clients and depots. The `uuid` parameter is optional.

#### With UUID Specified

When a UUID is specified, the macro generates a client that implements the trait directly, using the specified UUID for requests.

```rust
use anybus::{anybus_rpc, BusRiderRpc, BusRiderWithUuid};

#[anybus_rpc(uuid = "12345678-1234-1234-1234-123456789abc")]
pub trait MyService {
    async fn greet(&self, name: String) -> String;
}

// Implement the trait
pub struct MyServiceImpl;

#[async_trait::async_trait]
impl MyService for MyServiceImpl {
    async fn greet(&self, name: String) -> String {
        format!("Hello, {}!", name)
    }
}

// Create client and depot
let client = MyServiceClient::new(handle.clone()).await?;
let depot = MyServiceDepot { service: MyServiceImpl };

// Add the depot to Anybus
anybus.add_bus_depot(Box::new(depot), Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap()?);

// Use the client
let response = client.greet("World".to_string()).await?;
println!("{}", response); // "Hello, World!"
```

#### Without UUID Specified

When no UUID is specified, the client methods require an `endpoint_id` parameter, allowing dynamic endpoint selection.

```rust
use anybus::{anybus_rpc, BusRiderRpc};

#[anybus_rpc]
pub trait MyService {
    async fn greet(&self, name: String) -> String;
}

// Implement the trait
pub struct MyServiceImpl;

#[async_trait::async_trait]
impl MyService for MyServiceImpl {
    async fn greet(&self, name: String) -> String {
        format!("Hello, {}!", name)
    }
}

// Create client and depot
let client = MyServiceClient::new(handle.clone()).await?;
let depot = MyServiceDepot { service: MyServiceImpl };

// Choose your own UUID
let service_uuid = Uuid::new_v4();

// Add the depot to Anybus
anybus.add_bus_depot(Box::new(depot), service_uuid);

// Use the client with the endpoint ID
let response = client.greet("World".to_string(), service_uuid).await?;
println!("{}", response); // "Hello, World!"
```

### BusStop for Message Handling

Define a struct with the `#[anybus_stop]` macro for handling incoming messages:

```rust
use anybus::{anybus_stop, BusStop, BusTicket, BusRider, BusRiderWithUuid};

#[derive(Clone, Debug)]
struct MyMessage {
    content: String,
}

impl BusRider for MyMessage {}
impl BusRiderWithUuid for MyMessage {
    const ANYBUS_UUID: Uuid = uuid::from_u128(0x1234567890abcdef1234567890abcdef);
}

#[anybus_stop(uuid = "12345678-1234-1234-1234-123456789abc", message = MyMessage)]
struct MyStop;

impl BusStop<MyMessage> for MyStop {
    fn on_message(&self, message: MyMessage, handle: &Handle) -> Vec<BusTicket> {
        println!("Received: {}", message.content);
        // Process message and return tickets for responses if needed
        vec![]
    }
}

// Add the stop to Anybus
anybus.add_bus_stop(Box::new(MyStop), Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap())?;
```

## Architecture

### Core Concepts

- **Handle**: The main interface for sending and registering endpoints.
- **Endpoints**: Identified by UUIDs, can be unicast, anycast, broadcast, or RPC.
- **Packets**: Messages containing payload, sender, receiver, and reply information.
- **Routing**: Internal routing table manages message delivery.
- **Transports**: IPC and WebSocket managers handle network communication.

### Messaging Patterns

1. **Unicast**: One-to-one messaging to a specific endpoint.
2. **Anycast**: One-to-one messaging to any available endpoint of a type.
3. **Broadcast**: One-to-many messaging to all registered endpoints.
4. **RPC**: Request-response pattern with type-safe interfaces.

### Managed Services

- **BusDepot**: Managed RPC service that handles incoming requests.
- **BusStop**: Managed message handler for processing incoming messages.

## Advanced Usage

### Custom Transports

Anybus supports custom transports through the routing system. Implement the necessary traits to add new transport mechanisms.

### Error Handling

Anybus uses `AnyBusHandleError` for operation errors and `ReceiveError` for receiver-specific errors.

### Configuration

Use `AnyBusBuilder` to configure transports and options:

```rust
let anybus = AnyBus::build()
    .enable_ipc(true)
    .ws_remote(anybus::peers::WsRemoteOptions {
        url: "ws://example.com:8080".parse()?,
        realm: Realm::Global,
    })
    .init();
```

## Examples

See the `examples/` directory for complete working examples:

- `counter`: Simple counter service with IPC.
- `bridge`: WebSocket bridge example.
- `chat_tui`: Terminal chat application.

## Contributing

Contributions are welcome! Please see the GitHub repository for issues and pull requests.

## License

MIT License. See LICENSE file for details.