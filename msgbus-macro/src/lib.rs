use std::str::FromStr;

use proc_macro::TokenStream;
use std::fmt::Write;
use uuid::Uuid;
mod bus_uuid;

#[proc_macro_attribute]
pub fn bus_uuid(attr: TokenStream, mut item: TokenStream) -> TokenStream {
    bus_uuid::bus_uuid_impl(attr, item)
}
  