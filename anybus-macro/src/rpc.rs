use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{
    ItemTrait, Meta, Result, parse::Parse, parse::ParseStream, parse_macro_input, parse_quote,
};

struct Rpc2Attr {
    uuid: Option<u128>,
}

impl Parse for Rpc2Attr {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.is_empty() {
            return Ok(Self { uuid: None });
        }
        let meta: Meta = input.parse()?;
        match meta {
            Meta::NameValue(nv) if nv.path.is_ident("uuid") => {
                if let syn::Expr::Lit(lit) = nv.value {
                    if let syn::Lit::Str(s) = lit.lit {
                        let uuid_str = s.value();
                        let uuid = uuid::Uuid::parse_str(&uuid_str)
                            .map_err(|_| syn::Error::new(s.span(), "Invalid UUID"))?;
                        Ok(Self {
                            uuid: Some(uuid.as_u128()),
                        })
                    } else {
                        Err(syn::Error::new(lit.lit.span(), "Expected string literal"))
                    }
                } else {
                    Err(syn::Error::new(nv.value.span(), "Expected literal"))
                }
            }
            _ => Err(syn::Error::new(meta.span(), "Expected `uuid = \"...\"`")),
        }
    }
}

pub fn anybus_rpc_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr as Rpc2Attr);
    let trait_item = parse_macro_input!(item as ItemTrait);
    let trait_ident = &trait_item.ident;
    let mut methods = vec![];
    for item in &trait_item.items {
        if let syn::TraitItem::Fn(method) = item {
            let sig = &method.sig;
            let name = &sig.ident;
            let mut receiver = None;
            let mut args = vec![];
            for arg in &sig.inputs {
                match arg {
                    syn::FnArg::Receiver(recv) => {
                        receiver = Some(recv.clone());
                    }
                    syn::FnArg::Typed(pat_type) => {
                        if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                            args.push((pat_ident.ident.clone(), (*pat_type.ty).clone()));
                        }
                    }
                }
            }
            let ret = match &sig.output {
                syn::ReturnType::Default => quote!(()),
                syn::ReturnType::Type(_, ty) => quote!(#ty),
            };
            methods.push((name.clone(), receiver, args, ret));
        }
    }
    let request_ident = format_ident!("{}Request", trait_ident);
    let response_ident = format_ident!("{}Response", trait_ident);
    let client_ident = format_ident!("{}Client", trait_ident);
    let request_variants = methods.iter().map(|(name, _, args, _)| {
        let name_pascal = format_ident!(
            "{}",
            name.to_string()
                .to_uppercase()
                .chars()
                .next()
                .unwrap()
                .to_string()
                + &name.to_string()[1..]
        );
        if args.is_empty() {
            quote!( #name_pascal )
        } else {
            let fields = args.iter().map(|(arg_name, ty)| quote!( #arg_name: #ty ));
            quote!( #name_pascal { #(#fields),* } )
        }
    });
    let response_variants = methods.iter().map(|(name, _, _, ret)| {
        let name_pascal = format_ident!(
            "{}",
            name.to_string()
                .to_uppercase()
                .chars()
                .next()
                .unwrap()
                .to_string()
                + &name.to_string()[1..]
        );
        quote!( #name_pascal(#ret) )
    });
    let bus_rider_rpc_impl = quote! {
        impl ::anybus::BusRiderRpc for #request_ident {
            type Response = #response_ident;
        }
    };
    let bus_rider_with_uuid_impl = if let Some(uuid) = attr.uuid {
        quote! {
            impl ::anybus::BusRiderWithUuid for #request_ident {
                const ANYBUS_UUID: uuid::Uuid = uuid::Uuid::from_u128(#uuid);
            }
        }
    } else {
        quote!()
    };
    let client_methods = methods.iter().map(|(name, receiver, args, ret)| {
        let arg_names = args.iter().map(|(n, _)| n).collect::<Vec<_>>();
        let arg_tys = args.iter().map(|(_, t)| t).collect::<Vec<_>>();
        let request_variant = format_ident!(
            "{}",
            name.to_string()
                .to_uppercase()
                .chars()
                .next()
                .unwrap()
                .to_string()
                + &name.to_string()[1..]
        );
        let request_creation = if args.is_empty() {
            quote!( #request_ident::#request_variant )
        } else {
            quote!( #request_ident::#request_variant { #(#arg_names),* } )
        };
        let response_match = quote! {
            #response_ident::#request_variant(result) => result
        };
        let self_ref = if receiver.as_ref().map_or(false, |r| r.mutability.is_some()) {
            quote!(&mut self)
        } else {
            quote!(&self)
        };
        quote! {
            async fn #name(#self_ref, #(#arg_names: #arg_tys),*) -> Result<#ret, ::anybus::errors::AnyBusHandleError> {
                let request = #request_creation;
                let response = self.rpc_helper.request_to_uuid(request, self.endpoint_uuid).await?;
                let result = match response {
                    #response_match,
                    _ => panic!("Unexpected response variant"),
                };
                Ok(result)
            }
        }
    });
    let new_with_uuid = quote! {
        pub fn new_with_uuid(rpc_helper: ::anybus::RequestHelper, endpoint_uuid: uuid::Uuid) -> Self {
            Self { rpc_helper, endpoint_uuid }
        }
    };
    let new = if attr.uuid.is_some() {
        quote! {
            pub fn new(rpc_helper: ::anybus::RequestHelper) -> Self {
                Self::new_with_uuid(rpc_helper, <#request_ident as ::anybus::BusRiderWithUuid>::ANYBUS_UUID)
            }
        }
    } else {
        quote!()
    };
    let depot_match_arms = methods.iter().map(|(name, receiver, args, _)| {
        let arg_names = args.iter().map(|(n, _)| n).collect::<Vec<_>>();
        let request_variant = format_ident!(
            "{}",
            name.to_string()
                .to_uppercase()
                .chars()
                .next()
                .unwrap()
                .to_string()
                + &name.to_string()[1..]
        );
        let call = if receiver.as_ref().map_or(false, |r| r.mutability.is_some()) {
            quote!( self.#name(#(#arg_names),*).await )
        } else {
            quote!( self.#name(#(#arg_names),*).await )
        };
        if args.is_empty() {
            quote!( #request_ident::#request_variant => #response_ident::#request_variant(#call) )
        } else {
            quote!( #request_ident::#request_variant { #(#arg_names),* } => #response_ident::#request_variant(#call) )
        }
    });
    let depot_impl = quote! {
        impl ::anybus::BusDepot<#request_ident> for Box<dyn #trait_ident + Send + Sync> {
            async fn on_request(&mut self, request: Option<#request_ident>, _handle: &::anybus::Handle) -> #response_ident {
                match request.unwrap() {
                    #(#depot_match_arms),*
                }
            }
        }
    };
    let async_trait_attr: syn::Attribute = parse_quote!(#[async_trait::async_trait]);
    let mut modified_trait = trait_item.clone();
    modified_trait.attrs.push(async_trait_attr);
    let expanded = quote! {
        #modified_trait
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        pub enum #request_ident {
            #(#request_variants),*
        }
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        pub enum #response_ident {
            #(#response_variants),*
        }
        #bus_rider_rpc_impl
        #bus_rider_with_uuid_impl
        #[derive(Debug)]
        pub struct #client_ident {
            rpc_helper: ::anybus::RequestHelper,
            endpoint_uuid: uuid::Uuid,
        }
        impl #client_ident {
            #(#client_methods)*
            #new_with_uuid
            #new
        }
        #depot_impl
    };
    expanded.into()
}
