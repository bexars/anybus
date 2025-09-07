use proc_macro::TokenStream;
mod bus_uuid;

#[proc_macro_attribute]
pub fn bus_uuid(attr: TokenStream, item: TokenStream) -> TokenStream {
    bus_uuid::bus_uuid_impl(attr, item)
}
