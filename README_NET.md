# Anybus Network Setup: 3-Node Example

This guide demonstrates setting up a 3-node Anybus network with different transport configurations:

- **Node 1**: WebSocket listener + IPC (acts as a central hub)
- **Node 2**: IPC only (local communication)
- **Node 3**: WebSocket only (remote communication)

This setup allows local services to communicate via IPC and remote services to connect via WebSockets, with Node 1 bridging both networks.

## Prerequisites

- Rust 1.75+
- Three separate processes/machines (or simulated with different ports)

## Network Architecture

```
Node 1 (WS Listener + IPC)
├── WebSocket Server on port 8080
├── IPC endpoint
└── Bridges WS and IPC networks

Node 2 (IPC only)
└── Connects to Node 1 via IPC

Node 3 (WS only)
└── Connects to Node 1 via WebSocket
```

## Node 1: WebSocket Listener + IPC Hub

Node 1 acts as the central hub, running both a WebSocket server and IPC.

### Code Setup

```rust
use anybus::{AnyBus, BusRider, BusRiderWithUuid};
use uuid::Uuid;

#[derive(Clone, Debug)]
struct HubMessage {
    content: String,
    from_node: String,
}

impl BusRider for HubMessage {}
impl BusRiderWithUuid for HubMessage {
    const ANYBUS_UUID: Uuid = Uuid::from_u128(0x1234567890abcdef1234567890abcdef);
}

// Create Anybus with WS server and IPC
let mut anybus = AnyBus::build()
    .enable_ipc(true)
    .ws_server()
        .bind_addr("0.0.0.0:8080")
        .realm(anybus::Realm::Global)
    .init();

// Start the hub
anybus.run();

// Get handle for communication
let handle = anybus.handle().clone();

// Register a receiver for hub messages
let receiver = handle.listener()
    .broadcast::<HubMessage>()
    .register()
    .await?;

// Process messages from any node
while let Some(msg) = receiver.recv().await {
    println!("Hub received from {}: {}", msg.from_node, msg.content);
    
    // Broadcast to all connected nodes
    handle.send(HubMessage {
        content: format!("Broadcast: {}", msg.content),
        from_node: "hub".to_string(),
    }).await?;
}
```

### Running Node 1

```bash
cargo run --bin hub_node
```

## Node 2: IPC Only

Node 2 communicates locally with Node 1 via IPC.

### Code Setup

```rust
use anybus::{AnyBus, BusRider, BusRiderWithUuid};
use uuid::Uuid;

#[derive(Clone, Debug)]
struct LocalMessage {
    content: String,
}

impl BusRider for LocalMessage {}
impl BusRiderWithUuid for LocalMessage {
    const ANYBUS_UUID: Uuid = Uuid::from_u128(0xabcdef1234567890abcdef1234567890);
}

// Create Anybus with IPC only
let mut anybus = AnyBus::build()
    .enable_ipc(true)
    .init();

anybus.run();

let handle = anybus.handle().clone();

// Send messages to the hub
handle.send(HubMessage {
    content: "Hello from IPC node!".to_string(),
    from_node: "node2".to_string(),
}).await?;

// Receive broadcasts from hub
let receiver = handle.listener()
    .broadcast::<HubMessage>()
    .register()
    .await?;

while let Some(msg) = receiver.recv().await {
    println!("Node 2 received: {}", msg.content);
}
```

### Running Node 2

```bash
cargo run --bin ipc_node
```

## Node 3: WebSocket Only

Node 3 connects remotely to Node 1 via WebSocket.

### Code Setup

```rust
use anybus::{AnyBus, BusRider, BusRiderWithUuid};
use uuid::Uuid;

// Use the same message types as other nodes
// ... (HubMessage definition)

// Create Anybus with WebSocket remote connection
let mut anybus = AnyBus::build()
    .ws_remote()
        .url("ws://127.0.0.1:8080")  // Connect to Node 1
    .init();

anybus.run();

let handle = anybus.handle().clone();

// Send messages to the hub
handle.send(HubMessage {
    content: "Hello from WS node!".to_string(),
    from_node: "node3".to_string(),
}).await?;

// Receive broadcasts from hub
let receiver = handle.listener()
    .anycast::<HubMessage>()
    .register()
    .await?;

while let Some(msg) = receiver.recv().await {
    println!("Node 3 received: {}", msg.content);
}
```

### Running Node 3

```bash
cargo run --bin ws_node
```

## Communication Flow

1. **Node 2 (IPC)** sends a message to the hub endpoint
2. **Node 1 (Hub)** receives it via IPC, processes it, and broadcasts to all connected nodes
3. **Node 3 (WS)** receives the broadcast via WebSocket
4. **Node 3** can respond, which goes back through the hub to Node 2

## Configuration Options

### WebSocket Settings

```rust
.ws_server()
    .bind_addr("0.0.0.0:8080")
```

### IPC Settings

```rust
.enable_ipc(true)  // Uses default IPC socket
```

### Remote WebSocket

```rust
.ws_remote()
    .url("ws://example.com:8080")
```

## Routing and Endpoints

- **Unicast**: Direct one-to-one communication
- **Anycast**: Load-balanced one-to-many
- **Broadcast**: One-to-all within realm
- **RPC**: Request-response pattern

Messages are automatically routed based on endpoint UUIDs and transport availability.

## Error Handling

```rust
match anybus.run() {
    Ok(_) => println!("Network running"),
    Err(e) => eprintln!("Failed to start: {}", e),
}
```


## Scaling

- Add more IPC-only nodes for local clusters
- Add more WS-only nodes for remote clients
- Use multiple hub nodes with different realms for segmentation
- Implement custom transports for specialized networks

## Troubleshooting

- **Connection Issues**: Check firewall settings and port availability
- **IPC Not Working**: Ensure processes are on the same machine
- **WS Not Connecting**: Verify server is running and URL is correct
- **Messages Not Routing**: Check UUID consistency across nodes

This setup demonstrates Anybus's flexibility in creating hybrid local/remote networks with seamless communication between different transport types.