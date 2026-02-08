use std::str::FromStr;

use proc_macro::TokenStream;
use quote::quote;
use syn::{Ident, LitStr, Token, parse::Parse, parse::ParseStream};
use uuid::Uuid;

struct StopAttr {
    uuid: LitStr,
    message: Ident,
}

impl Parse for StopAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let _: Ident = input.parse()?;
        let _: Token![=] = input.parse()?;
        let uuid: LitStr = input.parse()?;
        let _: Token![,] = input.parse()?;
        let _: Ident = input.parse()?;
        let _: Token![=] = input.parse()?;
        let message: Ident = input.parse()?;
        Ok(StopAttr { uuid, message })
    }
}

pub(crate) fn anybus_stop_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_parsed: StopAttr = syn::parse(attr).unwrap();
    let uuid_str = attr_parsed.uuid.value();
    let _uuid = Uuid::from_str(&uuid_str).expect("Invalid UUID");
    let message_ident = attr_parsed.message;

    let struct_ast: syn::ItemStruct = syn::parse(item).unwrap();
    let struct_ident = &struct_ast.ident;

    let expanded = quote! {
        #struct_ast

        #[async_trait::async_trait]
        impl ::anybus::BusStopService for #struct_ident {
            fn on_load(&self, handle: &::anybus::Handle) -> Result<(), ::anybus::errors::AnyBusHandleError> {
                <Self as ::anybus::BusStop<#message_ident>>::on_load(self, handle)
            }

            fn on_shutdown(&self, handle: &::anybus::Handle) {
                <Self as ::anybus::BusStop<#message_ident>>::on_shutdown(self, handle)
            }

            async fn run(&self, handle: &::anybus::Handle, uuid: uuid::Uuid) -> Result<(), ::anybus::errors::AnyBusHandleError> {
                let receiver = handle.listener().unicast::<#message_ident>().endpoint(uuid).register().await?;
                loop {
                    match receiver.recv().await {
                        Ok(msg) => {
                            let tickets = <Self as ::anybus::BusStop<#message_ident>>::on_message(self, msg, handle);
                            for ticket in tickets {
                                let _ = handle.send_to_uuid(ticket.dest, ticket.rider);
                            }
                        }
                        Err(_) => break,
                    }
                }
                Ok(())
            }
        }
    };

    expanded.into()
}
