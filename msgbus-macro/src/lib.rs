use std::str::FromStr;

use proc_macro::TokenStream;
use uuid::Uuid;
use std::fmt::Write;

#[proc_macro_attribute]
pub fn bus_uuid(attr: TokenStream, mut item: TokenStream) -> TokenStream {
    let text = r#"
    fn as_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}
    "#;

    let mut buf = String::new();

    // println!("attr: \"{}\"", attr.to_string());
    let uuid = attr.to_string();
    let uuid = uuid.trim_matches('"');

    // println!("uuid: {uuid}");
    let uuid = Uuid::from_str(uuid).expect("Error decoding UUID");




    // println!("item: \"{}\"", item.to_string());
    let copy = item.clone();
    let name = copy.into_iter().skip(1).next().unwrap().to_string();
    // println!("name: {}", name);

    writeln!(buf, "impl msgbus::BusRider for {} {{", name).unwrap();
    writeln!(buf, "fn default_uuid(&self) -> Uuid {{").unwrap();
    writeln!(buf, "{}::get_uuid()", name).unwrap();
    writeln!(buf, "}}").unwrap();
    buf.push_str(text);
    writeln!(buf, "impl {} {{", name).unwrap();
    buf.push_str("const fn get_uuid() -> Uuid {\n");
    writeln!(buf, "const UUID: Uuid = Uuid::from_u128({});", uuid.as_u128()).unwrap();
    buf.push_str("UUID\n }\n }\n");

    // println!("buf: {buf}");



//     let new_stream:TokenStream = format!("impl msgbus::BusRider for {} {{\n
//     fn default_uuid(&self) -> Uuid {{\n
// {}::get_uuid(()\n}}\n{}", name, name, text).parse().unwrap();

//     let new_stream2:TokenStream = format!("impl {} {{
//     ", item);
    let new_stream:TokenStream = buf.parse().unwrap();

    item.extend(new_stream);
    // println!("newstream: {}", item);

    item
}


// mod ast_viz_debug;

// #[proc_macro]
// pub fn fn_macro_ast_viz_debug(input: TokenStream) -> TokenStream {
//   ast_viz_debug::fn_proc_macro_impl(input)
// }


