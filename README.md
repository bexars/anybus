# Anybus
A serverless Rust library/network service for easily exchanging messages with other parts of your program or other programs running both locally and on the web.  
## Core Functionality
You register an endpoint that is an annotated Struct or Enum.  You loop on the receive listener and wait for messages.  To send a message you simply send it through Handle.send().  The handle takes an Any (hence the name) and makes the routing decisions based on the Uuid of the endpoint.  There is also Rpc functionality where a response can be sent back via oneshot messages.  If the endpoint receives a Message it can't decode it will deliver it as an error with the full Vec<u8> of the serialized data as a payload.

### Endpoints
There are 3 main types of endpoints.
1. Unicast - Only one Uuid can be registered with this Id.
2. Anycast - Many endpoints can register the Uuid, but only one will receive a message.  This will prefer the closest one cost wise
3. Multicast/Broadcast - All registered Uuids will receive the message.

### Realms
When you register an endpoint you can select how reachable the endpoint is.  There are 4 main levels 
1. Process - No external routing, all messages will stay within the current running process
2. Userspace - This allows other programs running as the same user to interconnect via IPC
3. LocalNet - WIP - Not implemented, but will be for local Ethernet/Bluetooth/Wifi, etc. ( May just consider this global due to the way route propagation happens)
4. Global - Available to all connected nodes.  Currently implemented via WebSockets


### BusRider trait
All objects that traverse the bus need the BusRider trait.  It's automatically implemented for in-process types that are Clone + Send + Sync + Debug.  Types that cross a process or network hop also need serde::Serialize and serde::Deserialize (enabled by the `remote` / `ipc` / `ws` features).

### Rpc
#### Listening
* Register an Rpc endpoint of either Unicast or Anycast, .await an incoming request.  Take the request payload, form a response, and send the response back through the request object that you received.  

#### Sending
Instead of calling send() you call either rpc_once() or rpc_helper().
* rpc_once() is for single sends.  It registers a new return address per call.
* rpc_helper() is for when you need more than one rpc call.  It's a helper struct that maintains a unique Uuid in the network for all responses to be sent to.  
* You can call different endpoints with different types and the response will be of the associated return type of the request you made.  You only need one rpc_helper to make requests of all types.

### BusRiderRpc trait
A simple trait that marks what the expected return type will be when sending Rpc messages to this object

### bus_uuid("uuid") attribute
This is a default uuid for the object, it allows other codebases to have a well known Uuid when importing the Struct/Enum and wish to communicate with a well-known listener of that type.  Also allows simplified registration of the object as an endpoint without specifying a Uuid.  Just simply go to the Web and generate a new Uuid for any objects you want to be unique.  Good place to put a registration or rendezvous service that then hands out UUIDs of other endpoints for more complicated configurations

### Uuids
The system is agnostic to the type of Uuids that it can use for routing decisions.  It does use v7 ids internally, but mainly for ease of use and knowing which ones were created in which order.  All commands to register or send an object have a version that uses the provided Bus_Uuid of the object or a user supplied Uuid.

## Networking

### IPC
There is an IPC auto-discovery mechanism that could probably be published as it's own crate since there is no configured master server.  At runtime it tries to talk to master, if none exists it becomes master.  When a node connects to the master, it gets a list of all known nodes and connects to them as well.  If a node notices that it's master has died, it will try to become master.  

### Websocket
While a server socket must be configured, once the connection is established the protocol is agnostic to which side initiated the conversation.  It's the same protocol that runs over the IPC links at the message level.  They are not byte-similar and the byte layout is not well defined.  It's all serde and bincode inside binary Websocket messages.

#### Reconnect
The client will automatically try to reconnect to the server with a ^2 backoff timer (1s, 2s, 4s, etc).  Currently a 256s upper bound is set

### Routing
Routing is done via a Link State protocol similar to OSPF.  This allows a deterministic tree structure to be built and allow multicast message propagation with no duplicates.  Failover times are measured in milliseconds.  

#### Advertisements
Route advertisements are controlled by which Realm an endpoint is registered in.  Process won't be advertised, Userspace will only be advertised via IPC, and Global will be advertised to all connected nodes.  

## Logging
All logs are sent using the 'tracing' crate.  Not well organized and fairly messy, but you'll get some feedback.

## Examples
### chat-tui
A chat client built in ratatui that allows for serverless chat over IPC and Websockets.  Call with 'ws' option to make a websocket connection.  Use 'server' to listen on websocket. 'ipc' will listen on IPC.  --enable-ipc can be combined with 'ws' or 'server' to have both running
- This program will output logs in a ./logs/ directory off of the CWD
- Use /nick and /dm in the chat to change username and /dm <username> <msg>  to send a message to only one person
  
### relay 
This is a simple program that listens to both websocket and IPC connections for testing.  Can bridge chat-tui clients between websocket and IPC.  This takes a -c <config.toml> argument to use a config file for configuration of Anybus.  Look at the dummy folder in the relay example to see how it's setup

## Misc
### Wasm Support
Currently runs in a wasm browser instance.  IPC obviously doesn't work.  Websockets work well.   Can be configured to talk to two different servers at the same time for failover
### Dioxus Support
To use in dioxus and to use their spawn() enable the "dioxus" feature flag.

### Tokio re-export
An alias of either tokio_with_wasm or actual tokio is re-exported as anybus::tokio .  Use this to make your code easily be used in WASM or native architectures without a lot of cfg!() statements all over the place.

### Self-signed certs
Quick guide to making your own certs for WebSocket testing

https://stackoverflow.com/questions/60751795/unable-to-use-self-signed-certificates-with-tokio-rustls

### Alpha
This is very much a work in progress, but i've eaten the dogfood on some small projects and am relatively happy with how it's working

### Todo
- Local ipv6 and bluetooth auto-discovery and connectivity.  On Ethernet should it just peer with every neighbor, or do something smarter with multicast.  
- Non-websocket point-to-point connectivity???
- Make a service to handle event loop tasks.  You hand a bus_uuid object to Anybus and it runs the loop calling an attached closure
- [Mostly Done!!] A true RPC mechanism where you just call a function and it automagically puts the request on the bus and waits for the response.
- More examples needed, especially of web setups and of Rpc in action

### Demo server
- wss://turtle.trivarity.com:10800/
  - Run the chat_tui example and see if anyone responds!  I always keep a window open to it.  It's the current test server and it's probably a few revs behind but it shouldn't matter
- wss://turtle.trivarity.com:9798/

