use std::collections::HashMap;
use std::ffi::CString;
use std::fs::File;
use std::io::{Error,ErrorKind};
use std::process::exit;
use std::{vec, future};

use anyhow::Context;
use aya::maps::lpm_trie::Key;
use aya::maps::{LpmTrie, MapData, XskMap, HashMap as BpfHashMap};
use aya::programs::{Xdp, XdpFlags};
use aya::{include_bytes_aligned, Bpf};
use aya_log::BpfLogger;
use clap::{Parser, ValueEnum};
use log::{debug, error, info, warn, LevelFilter};
use tokio::signal;
use fabric_rs_config::{self, InstanceType};
use fabric_rs_common::{InterfaceConfig, RouteNextHop, InterfaceQueue};
use kube_virt_rs::flowtable::flowtable::MatchType;
use fabric_rs_userspace::{UserSpace, interface::interface::Interface};
use inquire::{error::CustomUserError, length, required, ui::RenderConfig, Text};
use env_logger::{Builder, Target};
use std::io::Write;


#[derive(Debug, Parser)]
struct Opt {
    #[clap(short, long)]
    config: String,
    #[clap(short, long)]
    log: Option<bool>,
    #[clap(short, long)]
    send: Option<bool>,
    #[clap(short, long)]
    dummy: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Config{
    id: u32,
    interfaces: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let opt = Opt::parse();


    let config = std::fs::read_to_string(opt.config)?;
    let config: Config = serde_yaml::from_str(&config)?;
    let target = Box::new(File::create(format!("/tmp/{}.log", config.id)).expect("Can't create file"));

    Builder::new()
        .format(|buf, record| {
            writeln!(
                buf,
                "{}:{} [{}] - {}",
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.level(),
                record.args()
            )
        })
        .target(env_logger::Target::Pipe(target))
        .filter(None, LevelFilter::Info)
        .init();
    
    let interfaces = config.interfaces.clone();
    let mut interface_list = HashMap::new();
    for interface in &interfaces{
        let interface = Interface::new(interface.to_string()).await?;
        interface_list.insert(interface.ifidx,interface);
    }

    // Bump the memlock rlimit. This is needed for older kernels that don't use the
    // new memcg based accounting, see https://lwn.net/Articles/837122/
    let rlim = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };
    let ret = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &rlim) };
    if ret != 0 {
        debug!("remove limit on locked memory failed, ret is: {}", ret);
    }

    // This will include your eBPF object file as raw bytes at compile-time and load it at
    // runtime. This approach is recommended for most real-world use cases. If you would
    // like to specify the eBPF program at runtime rather than at compile-time, you can
    // reach for `Bpf::load_file` instead.
    #[cfg(debug_assertions)]
    let mut bpf = Bpf::load(include_bytes_aligned!(
        "../../target/bpfel-unknown-none/debug/fabric-rs"
    ))?;
    #[cfg(not(debug_assertions))]
    let mut bpf = Bpf::load(include_bytes_aligned!(
        "../../target/bpfel-unknown-none/release/fabric-rs"
    ))?;
    if let Err(e) = BpfLogger::init(&mut bpf) {
        // This can happen if you remove all log statements from your eBPF program.
        warn!("failed to initialize eBPF logger: {}", e);
    }
    let program: &mut Xdp = bpf.program_mut("fabric_rs").unwrap().try_into()?;
    program.load()?;
    let mut interface_index_list = Vec::new();
    for interface in &interfaces{
        program.attach(interface, XdpFlags::default())
        .context("failed to attach the XDP program with default flags - try changing XdpFlags::default() to XdpFlags::SKB_MODE")?;
        let interface_index = get_interface_index(interface)?;
        interface_index_list.push(interface_index);
    }

    let route_table = if let Some(route_table) = bpf.take_map("ROUTINGTABLE"){
        let route_table_map: LpmTrie<MapData, u32, [RouteNextHop;32]> = LpmTrie::try_from(route_table).unwrap();
        route_table_map
    } else {
        panic!("ROUTINGTABLE map not found");
    };

    let xsk_map = if let Some(xsk_map) = bpf.take_map("XSKMAP"){
        let xsk_map: XskMap<MapData> = XskMap::try_from(xsk_map).unwrap();
        xsk_map
    } else {
        panic!("XSKMAP map not found");
    };

    let interface_queue_table = if let Some(interface_queue_table) = bpf.take_map("INTERFACEQUEUETABLE"){
        let interface_queue_table: BpfHashMap<MapData, InterfaceQueue, u32> = BpfHashMap::try_from(interface_queue_table).unwrap();
        interface_queue_table
    } else {
        panic!("INTERFACEQUEUETABLE map not found");
    };

    if !opt.dummy.unwrap_or(false){
        let mut jh_list = Vec::new();
        let mut user_space = UserSpace::new(
            interface_list,
            config.id,
            route_table,
        );
        let jh: tokio::task::JoinHandle<Result<(), anyhow::Error>> = tokio::spawn(async move {
            user_space.run(xsk_map, interface_queue_table, opt.send).await
        });
        jh_list.push(jh); 
        futures::future::join_all(jh_list).await;
    } else {
        let mut dummy_table = if let Some(dummy_table) = bpf.take_map("DUMMYTABLE"){
            let dummy_table: BpfHashMap<MapData,u32, u32> = BpfHashMap::try_from(dummy_table).unwrap();
            dummy_table
        } else {
            panic!("DUMMYTABLE map not found");
        };
        for (idx, _ ) in &interface_list {
            dummy_table.insert(idx, 1, 0).unwrap();
        }
    }
    info!("Waiting for Ctrl-C...");
    signal::ctrl_c().await?;
    info!("Exiting...");

    Ok(())
}

fn get_interface_index(interface_name: &str) -> Result<u32, Error> {
    let interface_name_cstring = CString::new(interface_name)?;
    let interface_index = unsafe { libc::if_nametoindex(interface_name_cstring.as_ptr()) };
    if interface_index == 0 {
        Err(Error::new(ErrorKind::NotFound, format!("Interface not found {}", interface_name)))
    } else {
        Ok(interface_index)
    }
}

/*
async fn cli(client: UserSpaceClient) -> anyhow::Result<()> {
    let jh = tokio::spawn(async move {
        loop{
            let command = Text::new("Cmd -> ")
            .with_autocomplete(&suggester)
            .with_validator(required!())
            //.with_validator(length!(10))
            .prompt()
            .unwrap();
    
            match command.as_str() {
                "RouteTable" => {
                    let route_table = client.get_route_table().await.unwrap();
                    println!("{}", route_table);
                },
                "NeighborTable" => {
                    let neighbor_table = client.get_neighbor_table().await.unwrap();
                    println!("{:?}", neighbor_table);
                },
                "ForwardingTable" => {
                    let forwarding_table = client.get_forwarding_table().await.unwrap();
                    println!("{:?}", forwarding_table);
                },
                "Exit" => {
                    exit(0)
                },
                _ => {
                    println!("Command not found");
                }
            }
            
        }
    });
    jh.await?;
    Ok(())
}
*/

fn suggester(val: &str) -> Result<Vec<String>, CustomUserError> {
    let suggestions = [
        "RouteTable",
        "NeighborTable",
        "ForwardingTable",
        "Exit"
    ];

    let val_lower = val.to_lowercase();

    Ok(suggestions
        .iter()
        .filter(|s| s.to_lowercase().contains(&val_lower))
        .map(|s| String::from(*s))
        .collect())
}

