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

    // let input = parse_macro_input!(item as ItemEnum);

    // let ItemEnum {
    //     // The visibility specifier of this function
    //     vis,

    //     // Other attributes applied to this function
    //     attrs,
    //     enum_token,
    //     ident,
    //     generics,
    //     brace_token,
    //     variants,
    // } = input;

    let uuid_u128_str = uuid.as_u128();

    let addon: TokenStream = quote!(

        impl msgbus::BusRiderWithUuid for #ident {
            const MSGBUS_UUID: uuid::Uuid = uuid::Uuid::from_u128(#uuid_u128_str);
        }

    )
    .into();

    orig.extend(addon);
    orig
}

// Old definition
// const UUID: Uuid = Uuid::from_u128(#uuid_u128_str);
// fn as_any(self: Box<Self>) -> Box<dyn std::any::Any> {
//         self
// }
// fn default_uuid(&self) -> Uuid {
//     #ident::get_uuid()
// }
// }

// impl #ident {
// const fn get_uuid() -> Uuid {
//     uuid::Uuid::from_u128(#uuid_u128_str)
// }
// }

// pub fn old_bus_uuid(attr: TokenStream, mut item: TokenStream) -> TokenStream {
//     let text = r#"
//     fn as_any(self: Box<Self>) -> Box<dyn std::any::Any> {
//         self
//     }
// }
//     "#;

//     let mut buf = String::new();

//     // println!("attr: \"{}\"", attr.to_string());
//     let uuid = attr.to_string();
//     let uuid = uuid.trim_matches('"');

//     // println!("uuid: {uuid}");
//     let uuid = Uuid::from_str(uuid).expect("Error decoding UUID");

//     // println!("item: \"{}\"", item.to_string());
//     let copy = item.clone();
//     let name = copy.into_iter().skip(1).next().unwrap().to_string();
//     // println!("name: {}", name);

//     writeln!(buf, "impl msgbus::BusRider for {} {{", name).unwrap();
//     writeln!(buf, "fn default_uuid(&self) -> Uuid {{").unwrap();
//     writeln!(buf, "{}::get_uuid()", name).unwrap();
//     writeln!(buf, "}}").unwrap();
//     buf.push_str(text);
//     writeln!(buf, "impl {} {{", name).unwrap();
//     buf.push_str("const fn get_uuid() -> Uuid {\n");
//     writeln!(
//         buf,
//         "const UUID: Uuid = Uuid::from_u128({});",
//         uuid.as_u128()
//     )
//     .unwrap();
//     buf.push_str("UUID\n }\n }\n");

//     // println!("buf: {buf}");

//     //     let new_stream:TokenStream = format!("impl msgbus::BusRider for {} {{\n
//     //     fn default_uuid(&self) -> Uuid {{\n
//     // {}::get_uuid(()\n}}\n{}", name, name, text).parse().unwrap();

//     //     let new_stream2:TokenStream = format!("impl {} {{
//     //     ", item);
//     let new_stream: TokenStream = buf.parse().unwrap();

//     item.extend(new_stream);
//     // println!("newstream: {}", item);

//     item
// }

// // mod ast_viz_debug;

// // #[proc_macro]
// // pub fn fn_macro_ast_viz_debug(input: TokenStream) -> TokenStream {
// //   ast_viz_debug::fn_proc_macro_impl(input)
// // }
