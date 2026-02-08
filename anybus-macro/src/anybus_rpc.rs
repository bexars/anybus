use std::str::FromStr;

use proc_macro::TokenStream;
use quote::quote;
use syn::{Ident, LitStr, Token, parse::Parse, parse::ParseStream};
use uuid::Uuid;

struct RpcAttr {
    uuid: LitStr,
}

impl Parse for RpcAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let _: Ident = input.parse()?;
        let _: Token![=] = input.parse()?;
        let uuid: LitStr = input.parse()?;
        Ok(RpcAttr { uuid })
    }
}

pub(crate) fn anybus_rpc_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_parsed: RpcAttr = syn::parse(attr).unwrap();
    let uuid_str = attr_parsed.uuid.value();
    let uuid = Uuid::from_str(&uuid_str).expect("Invalid UUID");
    let uuid_u128 = uuid.as_u128();

    let trait_ast: syn::ItemTrait = syn::parse(item).unwrap();
    let trait_ident = &trait_ast.ident;
    let client_ident = quote::format_ident!("{}Client", trait_ident);
    let depot_ident = quote::format_ident!("{}Depot", trait_ident);

    let mut request_structs = vec![];
    let mut client_methods = vec![];
    let mut depot_impls = vec![];

    for item in &trait_ast.items {
        if let syn::TraitItem::Fn(method) = item {
            let method_ident = &method.sig.ident;
            let request_ident = quote::format_ident!("{}Request", method_ident);

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

                impl ::anybus::BusRiderRpc for #request_ident {
                    type Response = #return_type;
                }

                impl ::anybus::BusRiderWithUuid for #request_ident {
                    const ANYBUS_UUID: uuid::Uuid = uuid::Uuid::from_u128(#uuid_u128);
                }
            };
            request_structs.push(request_struct);

            let client_method = quote! {
                async fn #method_ident(&self, #(#arg_names: #arg_types),*) -> #return_type {
                    self.handle.rpc_once(#request_ident { #(#arg_names),* }).await.unwrap()
                }
            };
            client_methods.push(client_method);

            let depot_impl = quote! {
                #[async_trait::async_trait]
                impl<T: #trait_ident + Send + Sync> ::anybus::BusDepot<#request_ident> for #depot_ident<T> {
                    async fn on_request(&self, mut request: ::anybus::RpcRequest<#request_ident>, _handle: &::anybus::Handle) -> #return_type {
                        let payload = request.payload().unwrap();
                        self.service.#method_ident(#(payload.#arg_names),*).await
                    }
                }
            };
            depot_impls.push(depot_impl);
        }
    }

    let expanded = quote! {
        #trait_ast

        #(#request_structs)*

        pub struct #client_ident {
            pub handle: ::anybus::Handle,
        }

        impl #trait_ident for #client_ident {
            #(#client_methods)*
        }

        pub struct #depot_ident<T: #trait_ident + Send + Sync> {
            pub service: T,
        }

        #(#depot_impls)*
    };

    expanded.into()
}
