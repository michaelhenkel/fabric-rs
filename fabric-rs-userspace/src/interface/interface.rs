use std::{ffi::CString, io::{Error,ErrorKind}, net::Ipv4Addr};
use futures::TryStreamExt;
use log::error;
use netlink_packet_route::link::LinkAttribute;
use rtnetlink::new_connection;


#[derive(Clone)]
pub struct Interface{
    pub name: String,
    pub ifidx: u32,
    pub ip: Ipv4Addr,
    pub mac: [u8;6],
}

impl Interface{
    pub async fn new(name: String) -> anyhow::Result<Self>{
        let ifidx = match get_interface_index(&name){
            Ok(ifidx) => ifidx,
            Err(e) => {
                return Err(anyhow::anyhow!("failed to get interface index: {:?}", e));
            }
        };
        let ip = match get_interface_ip(&name).await{
            Ok(ip) => ip,
            Err(e) => {
                return Err(anyhow::anyhow!("failed to get interface ip: {:?}", e));
            }
        };
        let mac = match get_local_mac(ifidx).await{
            Ok(Some(mac)) => mac,
            Ok(None) => {
                return Err(anyhow::anyhow!("failed to get local mac"));
            },
            Err(e) => {
                return Err(anyhow::anyhow!("failed to get local mac: {:?}", e));
            }
        };
        Ok(Self{
            name,
            ifidx,
            ip,
            mac,
        })
    }
}

async fn get_local_mac(index: u32) -> anyhow::Result<Option<[u8;6]>> {
    let (connection, handle, _) = new_connection().unwrap();
    tokio::spawn(connection);
    let mut links = handle.link().get().match_index(index).execute();
    let msg = if let Some(msg) = links.try_next().await? {
        msg
    } else {
        error!("no link with index {index} found");
        return Ok(None);
    };
    assert!(links.try_next().await?.is_none());

    for attr in msg.attributes.into_iter() {
        match attr{
            LinkAttribute::Address(addr) => {
                let array: [u8; 6] = addr.try_into().map_err(|v: Vec<u8>| {
                    anyhow::anyhow!("Expected a Vec of length {} but it was {}", 6, v.len())
                })?;
                return Ok(Some(array))
            },
            _ => {}
        }
    }
    Ok(None)
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

async fn get_interface_ip(interface_name: &str) -> anyhow::Result<Ipv4Addr>{
    let all_interfaces = pnet::datalink::interfaces();
    let interface = if let Some(interface) = all_interfaces
        .iter()
        .find(|e| e.name == interface_name.to_string()){
            interface
    } else {
        panic!("interface not found");
    };
    for ip in &interface.ips{
        if ip.is_ipv4(){
           match ip.ip(){
                std::net::IpAddr::V4(ip) => {
                    return Ok(ip);
                },
                _ => {}
            }
        }
    };    
    Err(anyhow::anyhow!("no ip found"))
}
