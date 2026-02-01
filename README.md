# Anybus
A serverless Rust library/network service for easily exchanging messages with other parts of your program or other programs running both locally and on the web.  
## Core Functionality
You register an endpoint that is an annotated Struct or Enum.  You loop on the receive listener and wait for messages.  To send a message you simply send it through Handle.send().  The handle takes an Any (hence the name) and makes the routing decisisions based on the Uuid of the endpoint.  There is also Rpc functionality where a response can be sent back via oneshot messages

### Endpoints
There are 3 main types of endpoints.
1. Unicast - Only one Uuid can be registered with this Id.
2. Anycast - Many endpoints can register the Uuid, but only one will receive a message.  This will prefer the closest one cost wise
3. Broadcast - All registered Uuids will receive the message.

### Realms
When you register an endpoint you can select how reachable the endpoint is.  There are 4 main levels 
1. Process - No external routing, all messages will stay within the current running process
2. Userspace - This allows other programs running as the same user to interconnect via IPC
3. LocalNet - WIP - Not implemented, but will be for local Ethernet/Bluetooth/Wifi, etc. ( May just consider this global due to the way route propagation happens)
4. Global - Available to all connected nodes.  Currently implemented via WebSockets


### BusRider trait
All objects that traverse the network need the BusRider trait.  It's automatically implemented for all types that are Clone and serializable with serde::Serialize and serde::Deserialize

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
This is a default uuid for the object, it allows other codebases to have a well known Uuid when importing the Struct/Enum and wish to communicate with a well-known listener of that type.  Also allows simplified registration of the object as an endpoint without specifying a Uuid.  Just simply go to the Web and generate a new Uuid for any objects you want to be unique.

### Uuids
The system is agnostic to the type of Uuids that it can use for routing decisions.  It does use v7 ids internally, but mainly for ease of use and knowing which ones were created in which order.  All commands to register or send an object have a version that uses the provided Bus_Uuid of the object or a user supplied Uuid.

## Networking

### IPC
There is an IPC auto-discovery mechanism that could probably be published as it's own crate since there is no configured master server.  At runtime it tries to talk to master, if none exists it becomes master.  When a node connects to the master, it gets a list of all known nodes and connects to them as well.  If a node notices that it's master has died, it will try to become master.  

### Websocket
While a server socket must be configured, once the connection is established the protocol is agnostic to which side initiated the conversation.  It's the same protocol that runs over the IPC links at the message level.  They are not byte-similar and the byte layout is not well defined.  It's all serde and cbor inside Byte Websocket messages.

#### Reconnect
The client will automatically try to reconnect to the server with a ^2 backoff timer (1s, 2s, 4s, etc).  Currently no upper bound is set

### Routing
There is cobbled together routing algorithm that allows for network loops to be created without causing broadcast storms.  This probably has some odd bugs in it, but I've successfully created loops between three IPC instances and several Websocket connections with no issues

#### Advertisements
Route advertisements are controlled by which Realm an endpoint is registered in.  Process won't be advertised, Userspace will only be advertised via IPC, and Global will be advertised to all connected nodes.  Route propagation is done in a tree fashion to prevent routing loops.

## Examples
### chat-tui
A chat client built in ratatui that allows for serverless chat over IPC and Websockets.  Very basic right now, but to configure the websocket endpoints, edit the file directly.  Call with 'ws' option to make a websocket connection.  Use 'server' to listen and no option will listen on IPC
- This program will output logs in a ./logs/ directory off of the CWD
### bridge 
This is a simple program that listens to both websocket and IPC connections for testing.  Can bridge chat-tui clients between websocket and IPC.  

## Misc
### Wasm Support
Currently runs in a wasm browser instance.  IPC obviously doesn't work.  Websockets are a WIP.  I've managed to get it running in Egui, Dioxus and some other web enabled frameworks

### Self-signed certs
Quick guide to making your own certs for WebSocket testing

https://stackoverflow.com/questions/60751795/unable-to-use-self-signed-certificates-with-tokio-rustls

### Alpha
This is very much a work in progress, but i've eaten the dogfood on some small projects and am relatively happy with how it's working

### Todo
- Local ipv6 and bluetooth auto-discovery and connectivity.  On Ethernet should it just peer with every neighbor, or do something smarter with multicast.  
- Non-websocket point-to-point connectivity???
- Make a service to handle event loop tasks.  You hand a bus_uuid object to Anybus and it runs the loop calling an attached closure
- Possibly a true RPC mechanism where you just call a function and it automagically puts the request on the bus and waits for the response.
- More examples needed, especially of web setups and of Rpc in action
