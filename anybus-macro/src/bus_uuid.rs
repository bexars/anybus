use std::str::FromStr;

use proc_macro::TokenStream;
use quote::quote;
use syn::DeriveInput;
use uuid::Uuid;

pub(crate) fn bus_uuid_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut orig = item.clone();

    let uuid = attr.to_string();
    let uuid = uuid.trim_matches('"');
    let uuid = Uuid::from_str(uuid).expect("Error decoding UUID");

    let ast: DeriveInput = syn::parse(item).unwrap();

    let DeriveInput { ident, .. } = ast;

    let uuid_u128_str = uuid.as_u128();

    let addon: TokenStream = quote!(

        impl ::anybus::BusRiderWithUuid for #ident {
            const MSGBUS_UUID: uuid::Uuid = uuid::Uuid::from_u128(#uuid_u128_str);
        }

    )
    .into();

    orig.extend(addon);
    orig
}
