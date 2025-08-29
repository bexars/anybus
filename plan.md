# Current Needs
* DONE - IPC needs to restart rendezvous service
* DONE - Listen on your own port for conns
* Connect to other ports and share known peers
* Delinting and TODO cleanup


Must implement
* determine which interfaces to listen on
  * https://docs.rs/interfaces/latest/  - looks good for weeding out some interfaces
* ospf-ish service advertising


Nice to have
* watch for interface changes - https://github.com/mxinden/if-watch

Anti-goals
* no stringified messaging, everything in a struct or enum that's converted to wire format
* no centralized lookup - routing is from smart edges
