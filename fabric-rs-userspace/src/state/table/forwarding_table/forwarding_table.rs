use aya::maps::{lpm_trie::Key, LpmTrie, MapData};
use fabric_rs_common::RouteNextHop as RNH;
use std::{fmt::Display, net::Ipv4Addr, sync::{Arc, Mutex}};
use super::super::table::{Table, TableKey, TableValue};

#[derive(Clone)]
pub struct ForwadingTableKey(u32,u32);

impl From<(u32,u32)> for ForwadingTableKey{
    fn from((prefix, prefix_len): (u32,u32)) -> Self {
        ForwadingTableKey(prefix, prefix_len)
    }
}

impl Into<(u32,u32)> for ForwadingTableKey{
    fn into(self) -> (u32,u32) {
        (self.0, self.1)
    }
}

#[derive(Clone)]
pub struct ForwadingTableValue([RNH;32]);

impl From<[RNH;32]> for ForwadingTableValue{
    fn from(next_hops: [RNH;32]) -> Self {
        ForwadingTableValue(next_hops)
    }
}

impl Into<[RNH;32]> for ForwadingTableValue{
    fn into(self) -> [RNH;32] {
        self.0
    }
}

impl TableKey for ForwadingTableKey{}
impl TableValue for ForwadingTableValue{}

impl <K: TableKey,V: TableValue>Table<K,V> for ForwardingTable
where
    K: Into<(u32,u32)> + From<(u32,u32)>,
    V: Into<[RNH;32]> + From<[RNH;32]>,
{
    fn get(&self, k: K) -> Option<V>{
        let entries = self.entries.lock().unwrap();
        let (prefix, prefix_len):(u32,u32) = k.into();
        let key = Key::new(prefix_len, prefix);
        match entries.get(&key, 0){
            Ok(entry) => {
                Some(entry.into())
            },
            Err(_) => None,
        }
    }

    fn list(&self) -> Vec<(K,V)>{
        let entries = self.entries.lock().unwrap();
        let mut list = Vec::new();
        for entry in entries.iter(){
            let (key, rnh) = entry.unwrap();
            let k: K = (key.data(), key.prefix_len() as u32).into();
            let v: V = rnh.clone().into();
            
            list.push((k,v));
        }
        list
    }

    fn add(&mut self, k: K, v: V){
        let mut entries = self.entries.lock().unwrap();
        let (prefix, prefix_len):(u32,u32) = k.into();
        let key = Key::new(prefix_len, prefix);
        entries.insert(&key, v.into(), 0);
    }

    fn remove(&mut self, k: K){
        let mut entries = self.entries.lock().unwrap();
        let (prefix, prefix_len):(u32,u32) = k.into();
        let key = Key::new(prefix_len, prefix);
        entries.remove(&key);
    }
}


#[derive(Debug)]
pub struct ForwardingTable{
    entries: Arc<Mutex<LpmTrie<MapData, u32, [RNH;32]>>>
}

impl ForwardingTable
{
    pub fn new<K,V>(entries: Arc<Mutex<LpmTrie<MapData, u32, [RNH;32]>>>) -> Box<dyn Table<K,V>>
    where
        K: Into<(u32,u32)> + From<(u32,u32)> + TableKey,
        V: Into<[RNH;32]> + From<[RNH;32]> + TableValue,
    {
        Box::new(ForwardingTable{
            entries
        })
    }
}


impl Display for ForwardingTable{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut entries = "".to_string();
        for entry in self.entries.lock().unwrap().iter(){
            let (key, rnh) = entry.unwrap();
            let entry = Entry::from((key.data(), key.prefix_len() as u8, rnh));
            entries = format!("{}\n{}", entries, entry);
        }
        write!(f, "{}", entries)
    }
}

#[derive(Debug)]
pub struct Entry{
    pub prefix: Ipv4Addr,
    pub prefix_len: u8,
    pub next_hops: [RNH;32],
}

impl From<(u32, u8, [RNH;32])> for Entry{
    fn from((prefix, prefix_len, next_hops): (u32, u8, [RNH;32])) -> Self {
        Entry{
            prefix: Ipv4Addr::from(prefix),
            prefix_len,
            next_hops,
        }
    }
}

impl Display for Entry{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix = self.prefix;
        let prefix_len = self.prefix_len;
        let mut next_hops = "".to_string();
        for next_hop in &self.next_hops{
            next_hops = format!("{}\n\t\t\t{:?}", next_hops, next_hop);
        }
        write!(f, "prefix: {}, prefix_len: {}, next_hops: {}", prefix, prefix_len, next_hops)
    }
}