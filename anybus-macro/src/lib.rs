use proc_macro::TokenStream;

mod bus_uuid;

mod rpc;

#[proc_macro_attribute]
pub fn bus_uuid(attr: TokenStream, item: TokenStream) -> TokenStream {
    bus_uuid::bus_uuid_impl(attr, item)
}

#[proc_macro_attribute]
pub fn anybus_rpc(attr: TokenStream, item: TokenStream) -> TokenStream {
    rpc::anybus_rpc_impl(attr, item)
}
