use std::{collections::HashMap, fmt::Display, net::Ipv4Addr};
use pnet::util::MacAddr;
use super::super::table::{Table, TableKey, TableValue};

#[derive(Clone)]
pub struct NeighborTableKey(u32);

impl From<u32> for NeighborTableKey{
    fn from(ip: u32) -> Self {
        NeighborTableKey(ip)
    }
}

impl Into<u32> for NeighborTableKey{
    fn into(self) -> u32 {
        self.0
    }
}

#[derive(Clone)]
pub struct NeighborTableValue(Neighbor);

impl From<Neighbor> for NeighborTableValue{
    fn from(neighbor: Neighbor) -> Self {
        NeighborTableValue(neighbor)
    }
}

impl Into<Neighbor> for NeighborTableValue{
    fn into(self) -> Neighbor {
        self.0
    }
}

impl TableKey for NeighborTableKey{}
impl TableValue for NeighborTableValue{}

impl <K: TableKey,V: TableValue>Table<K,V> for NeighborTable
where
    K: Into<u32> + From<u32>,
    V: Into<Neighbor> + From<Neighbor>,
{
    fn get(&self, k: K) -> Option<V>{
        let k: u32 = k.into();
        self.entries.get(&k).map(|v| v.clone().into())
    }

    fn list(&self) -> Vec<(K,V)>{
        let mut list = Vec::new();
        for (k, v) in &self.entries{
            let k: K = (*k).into();
            let v: V = v.clone().into();
            list.push((k,v));
        }
        list
    }

    fn add(&mut self, k: K, v: V){
        let k: u32 = k.into();
        self.entries.insert(k, v.into());
    }

    fn remove(&mut self, k: K){
        let k: u32 = k.into();
        self.entries.remove(&k);
    }
}

#[derive(Debug, Clone)]
pub struct NeighborTable{
    entries: HashMap<u32, Neighbor>
}

impl NeighborTable
{
    pub fn new<K,V>() -> Box<dyn Table<K,V>>
    where
        K: Into<u32> + From<u32> + TableKey,
        V: Into<Neighbor> + From<Neighbor> + TableValue,
    {
        Box::new(NeighborTable{
            entries: HashMap::new()
        })
    }
}

#[derive(Debug, Clone)]
pub struct Neighbor{
    interfaces: HashMap<[u8;6], NeighborInterface>
}

impl Neighbor
{
    pub fn new() -> Neighbor{
        Neighbor{
            interfaces: HashMap::new()
        }
    }
    pub fn add_interface(&mut self, interface: NeighborInterface){
        self.interfaces.insert(interface.neighbor_mac, interface);
    }
}

impl Display for Neighbor{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        for (mac, interface) in &self.interfaces{
            let mac = MacAddr::from(*mac);
            s.push_str(&format!("\n\t\tmac: {}\n\t\tinterface: {}", mac.to_string(), interface));
        }
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone)]
pub struct NeighborInterface{
    neighbor_ip: u32,
    neighbor_mac: [u8;6],
    local_ip: u32,
    local_mac: [u8;6],
    local_ifidx: u32,
}

impl NeighborInterface{
    pub fn new(neighbor_ip: u32, neighbor_mac: [u8;6], local_ip: u32, local_mac: [u8;6], local_ifidx: u32) -> NeighborInterface{
        NeighborInterface{
            neighbor_ip,
            neighbor_mac,
            local_ip,
            local_mac,
            local_ifidx,
        }
    }
}

impl Display for NeighborInterface{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let neighbor_mac = MacAddr::from(self.neighbor_mac);
        let local_mac = MacAddr::from(self.local_mac);
        let neighbor_ip: Ipv4Addr = self.neighbor_ip.into();
        let local_ip: Ipv4Addr = self.local_ip.into();
        write!(f, "\n\t\t\tneighbor_ip: {}\n\t\t\tneighbor_mac: {}\n\t\t\tlocal_ip: {}\n\t\t\tlocal_mac: {}\n\t\t\tlocal_ifidx: {}", neighbor_ip, neighbor_mac.to_string(), local_ip, local_mac.to_string(), self.local_ifidx)
    }
}
