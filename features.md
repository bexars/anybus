# Features

AnyBus is usable as an in-process bus with the default feature set. IPC and WebSocket are opt-in and pull in serialization.

```
default = tokio
                ┌──► serde ──► uuid/serde, anybus-macro/serde
ipc ──► remote ─┤
ws  ──► remote ─┴──► bincode
          ▲
ws_server ┘
ws_rustls (TLS stack for native WebSocket)

dioxus (spawn helper only; does not enable remote or serde)
```

## `tokio` (default)

Async runtime. This is the only default feature.

On native targets Tokio is always available. On `wasm32` it is optional and enabled by this feature.

## `remote`

Shared machinery for talking to other processes or hosts:

- Serde bounds on message types (`Serialize` / `Deserialize`)
- `BusRider::encode_payload()` and bincode encode/decode
- Peer registration, advertisements, and `WirePacket`

You normally enable this through `ipc` or `ws`, not directly.

Enables: `tokio`, `serde`, `bincode`

## `serde`

Turns on the `serde` crate and serde support in `uuid` and `anybus-macro`.

With this feature (which `remote` enables):

- Message types that leave the process must implement `Serialize` + `Deserialize`
- `anybus_rpc` emits those derives on generated request/response enums
- Wire types (`Address`, `EndpointId`, `Realm`, …) gain serde impls

Without it, in-process types only need `Clone + Send + Sync + Debug`.

## `ipc`

Unix/Windows local-socket transport with auto-discovery (one process becomes master, others mesh to it).

Enables: `remote` (and therefore `serde` + `bincode`)

## `ws`

WebSocket client. Same message-level protocol as IPC; frames are bincode in binary WebSocket messages. Includes reconnect with exponential backoff.

Enables: `remote`, `ws_rustls`

## `ws_server`

Listen for incoming WebSocket connections. Implies `ws`.

## `ws_rustls`

Native TLS stack for WebSocket (`tokio-rustls` / `rustls`). Pulled in by `ws`; you do not need to enable it yourself.

## `dioxus`

Uses Dioxus’s `spawn` instead of Tokio’s. Does **not** enable `serde` or `remote`.

## Message types

| Build | What a bus message needs |
|---|---|
| default / `tokio` only | `Clone + Send + Sync + Debug` |
| `ipc`, `ws`, or `remote` | plus `Serialize` + `Deserialize` |

Local delivery is still a downcast. Serde is only used when a payload is encoded for a remote hop.

## Typical combinations

```toml
# In-process only
anybus = "0.2"

# Local processes
anybus = { version = "0.2", features = ["ipc"] }

# Outbound WebSocket
anybus = { version = "0.2", features = ["ws"] }

# Bridge / listen + IPC (chat_tui, bridge examples)
anybus = { version = "0.2", features = ["ipc", "ws", "ws_server"] }
```
