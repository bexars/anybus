use std::str::FromStr;

use proc_macro::TokenStream;
use quote::quote;
use syn::{Ident, LitStr, Token, parse::Parse, parse::ParseStream};
use uuid::Uuid;

struct RpcAttr {
    uuid: Option<LitStr>,
}

impl Parse for RpcAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let uuid = if input.is_empty() {
            None
        } else {
            let _: Ident = input.parse()?;
            let _: Token![=] = input.parse()?;
            Some(input.parse()?)
        };
        Ok(RpcAttr { uuid })
    }
}

pub(crate) fn anybus_rpc_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_parsed: RpcAttr = syn::parse(attr).unwrap();

    let trait_ast: syn::ItemTrait = syn::parse(item).unwrap();
    let trait_ident = &trait_ast.ident;
    let has_uuid = attr_parsed.uuid.is_some();
    let uuid = if let Some(uuid_lit) = attr_parsed.uuid {
        Uuid::from_str(&uuid_lit.value()).expect("Invalid UUID")
    } else {
        // Use a fixed default UUID when not specified
        Uuid::from_u128(0x12345678_1234_1234_1234_123456789abc)
    };
    let uuid_u128 = uuid.as_u128();
    let client_ident = quote::format_ident!("{}Client", trait_ident);
    let depot_ident = quote::format_ident!("{}Depot", trait_ident);
    let request_enum_ident = quote::format_ident!("{}Request", trait_ident);
    let response_enum_ident = quote::format_ident!("{}Response", trait_ident);

    let uuid_impl = if has_uuid {
        quote! {
            impl ::anybus::BusRiderWithUuid for #request_enum_ident {
                const ANYBUS_UUID: uuid::Uuid = uuid::Uuid::from_u128(#uuid_u128);
            }
        }
    } else {
        quote! {}
    };

    let mut request_structs = vec![];
    let mut request_variants = vec![];
    let mut response_variants = vec![];
    let mut client_methods = vec![];
    let mut depot_match_arms = vec![];

    for item in &trait_ast.items {
        if let syn::TraitItem::Fn(method) = item {
            let method_ident = &method.sig.ident;
            let request_ident = quote::format_ident!("{}{}Request", trait_ident, method_ident);

            let mut fields = vec![];
            let mut arg_names = vec![];
            let mut arg_types = vec![];

            for input in method.sig.inputs.iter().skip(1) {
                // skip &self
                if let syn::FnArg::Typed(pat_type) = input {
                    if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                        let field_name = &pat_ident.ident;
                        let ty = &pat_type.ty;
                        fields.push(quote! { pub #field_name: #ty });
                        arg_names.push(quote! { #field_name });
                        arg_types.push(ty.clone());
                    }
                }
            }

            let return_type = match &method.sig.output {
                syn::ReturnType::Type(_, ty) => ty.as_ref(),
                syn::ReturnType::Default => &syn::parse_quote! { () },
            };

            let request_struct = quote! {
                #[derive(Clone, Debug)]
                #[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
                pub struct #request_ident {
                    #(#fields),*
                }
            };
            request_structs.push(request_struct);

            request_variants.push(quote! { #method_ident(#request_ident) });
            response_variants.push(quote! { #method_ident(#return_type) });

            let client_method = if has_uuid {
                quote! {
                    async fn #method_ident(&mut self, #(#arg_names: #arg_types),*) -> #return_type {
                        match self.helper.request(#request_enum_ident::#method_ident(#request_ident { #(#arg_names),* })).await.unwrap() {
                            #response_enum_ident::#method_ident(res) => res,
                            _ => panic!("Unexpected response variant"),
                        }
                    }
                }
            } else {
                quote! {
                    async fn #method_ident(&mut self, #(#arg_names: #arg_types,)* endpoint_id: uuid::Uuid) -> #return_type {
                        match self.helper.request_to_uuid(#request_enum_ident::#method_ident(#request_ident { #(#arg_names),* }), endpoint_id).await.unwrap() {
                            #response_enum_ident::#method_ident(res) => res,
                            _ => panic!("Unexpected response variant"),
                        }
                    }
                }
            };
            client_methods.push(client_method);

            depot_match_arms.push(quote! {
                #request_enum_ident::#method_ident(payload) => {
                    let response = self.service.#method_ident(#(payload.#arg_names),*).await;
                    #response_enum_ident::#method_ident(response)
                }
            });
        }
    }

    let client_impl = quote! { #(#client_methods)* };

    let expanded = quote! {
        #trait_ast

        #(#request_structs)*

        #[derive(Clone, Debug)]
        #[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
        pub enum #request_enum_ident {
            #(#request_variants),*
        }

        #[derive(Clone, Debug)]
        #[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
        pub enum #response_enum_ident {
            #(#response_variants),*
        }

        impl ::anybus::BusRiderRpc for #request_enum_ident {
            type Response = #response_enum_ident;
        }

        #uuid_impl

        pub struct #client_ident {
            pub handle: ::anybus::Handle,
            helper: ::anybus::RequestHelper,
        }

        impl #client_ident {
            pub async fn new(handle: ::anybus::Handle) -> Self {
                let helper = handle.rpc_helper().await.unwrap();
                Self { handle, helper }
            }
        }

        impl #trait_ident for #client_ident {
            #client_impl
        }

        pub struct #depot_ident<T: #trait_ident + Send + Sync> {
            pub service: T,
        }

        #[async_trait::async_trait]
        impl<T: #trait_ident + Send + Sync> ::anybus::BusDepotService for #depot_ident<T> {
            fn on_load(&self, handle: &::anybus::Handle) -> Result<(), ::anybus::errors::AnyBusHandleError> {
                self.service.on_load(handle)
            }

            fn on_shutdown(&self, handle: &::anybus::Handle) {
                self.service.on_shutdown(handle)
            }

            async fn run(&self, handle: &::anybus::Handle, uuid: uuid::Uuid) -> Result<(), ::anybus::errors::AnyBusHandleError> {
                let receiver = handle.listener().rpc().endpoint(uuid).register::<#request_enum_ident>()?;
                loop {
                    match receiver.recv().await {
                        Ok(request) => {
                            let response = match request.payload().unwrap() {
                                #(#depot_match_arms),*
                            };
                            request.reply(response);
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
