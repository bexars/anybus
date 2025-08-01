#[tokio::main]
async fn main() {
    let addrs = if_addrs::get_if_addrs().unwrap();
    // dbg!(&addrs);
    addrs
        .into_iter()
        .filter(|a| a.addr.is_link_local())
        .filter(|a| !a.name.starts_with("lo"))
        .filter(|a| !a.name.contains("tun"))
        .for_each(|a| {
            println!("Index: {:2} {:5} {:?}", a.index.unwrap(), a.name, a.addr);
        })
}
