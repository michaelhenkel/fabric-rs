use std::{collections::HashMap, fmt::Display, net::Ipv4Addr};
use super::super::table::{Table, TableKey, TableValue};
use pnet::util::MacAddr;

#[derive(Clone)]
pub struct RoutingTableKey(u32,u8);

impl From<(u32,u8)> for RoutingTableKey{
    fn from((prefix, prefix_len): (u32,u8)) -> Self {
        RoutingTableKey(prefix, prefix_len)
    }
}

impl Into<(u32,u8)> for RoutingTableKey{
    fn into(self) -> (u32,u8) {
        (self.0, self.1)
    }
}

#[derive(Clone)]
pub struct RoutingTableValue(Route);

impl From<Route> for RoutingTableValue{
    fn from(route: Route) -> Self {
        RoutingTableValue(route)
    }
}

impl Into<Route> for RoutingTableValue{
    fn into(self) -> Route {
        self.0
    }
}

impl TableKey for RoutingTableKey{}
impl TableValue for RoutingTableValue{}

impl <K: TableKey,V: TableValue>Table<K,V> for RouteTable
where
    K: Into<(u32,u8)> + From<(u32,u8)>,
    V: Into<Route> + From<Route>,
{
    fn get(&self, k: K) -> Option<V>{
        let (prefix, prefix_len):(u32,u8) = k.into();
        self.entries.get(&(prefix,prefix_len)).map(|v| v.clone().into())
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
        let (prefix, prefix_len):(u32,u8) = k.into();
        self.entries.insert((prefix,prefix_len), v.into());
    }

    fn remove(&mut self, k: K){
        let (prefix, prefix_len):(u32,u8) = k.into();
        self.entries.remove(&(prefix,prefix_len));
    }
}

#[derive(Debug, Clone)]
pub struct RouteTable{
    entries: HashMap<(u32,u8), Route>
}

impl RouteTable
{
    pub fn new<K,V>() -> Box<dyn Table<K,V>>
    where
        K: Into<(u32,u8)> + From<(u32,u8)> + TableKey,
        V: Into<Route> + From<Route> + TableValue,
    {
        Box::new(RouteTable{
            entries: HashMap::new()
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Route{
    pub origin: RouteOrigin,
    pub next_hops: RouteNextHopList
}

impl Route{
    pub fn new(origin: RouteOrigin, next_hops: RouteNextHopList) -> Self{
        Route{
            origin,
            next_hops
        }
    }
    pub fn get_next_hops(&self) -> &Vec<RouteNextHop>{
        &self.next_hops.0
    }
}

impl Display for Route{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\n\t\torigin: {}\n\t\tnext_hops: {}\n", self.origin, self.next_hops)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RouteOrigin{
    LOCAL,
    REMOTE,
}
impl Display for RouteOrigin{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self{
            RouteOrigin::LOCAL => write!(f, "LOCAL"),
            RouteOrigin::REMOTE => write!(f, "REMOTE"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteNextHop{
    pub originator_id: u32,
    pub ip: u32,
    pub hops: u32,
    pub ifidx: u32,
    pub src_mac: [u8;6],
    pub dst_mac: [u8;6],
}
impl Display for RouteNextHop{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let src_mac = MacAddr::from(self.src_mac);
        let dst_mac = MacAddr::from(self.dst_mac);
        let ip: Ipv4Addr = self.ip.into();
        write!(f, "\n\t\t\toriginator_id: {}\n\t\t\tgw: {}\n\t\t\thops: {}\n\t\t\tifidx: {}\n\t\t\tsrc_mac: {}\n\t\t\tdst_mac: {}", self.originator_id, ip, self.hops, self.ifidx, src_mac.to_string(), dst_mac.to_string())
    }
}

impl RouteNextHop{
    pub fn new(originator_id: u32, ip: u32, hops: u32, ifidx: u32, src_mac: [u8;6], dst_mac: [u8;6]) -> Self{
        RouteNextHop{
            originator_id,
            ip,
            hops,
            ifidx,
            src_mac,
            dst_mac,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteNextHopList(Vec<RouteNextHop>);

impl RouteNextHopList{
    pub fn new() -> Self{
        RouteNextHopList(Vec::new())
    }
    pub fn add(&mut self, nh: RouteNextHop){
        self.0.push(nh);
    }
}

impl Display for RouteNextHopList{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        for nh in &self.0{
            s.push_str(&nh.to_string());
            s.push_str("\n");
        }
        write!(f, "{}", s)
    }
}