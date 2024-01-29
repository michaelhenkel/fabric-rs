use std::{fmt::{Display, Formatter}, net::Ipv4Addr, sync::{Arc, Mutex}};
use aya::maps::{LpmTrie, MapData};
use log::{error, info};
use fabric_rs_common::RouteNextHop as RNH;
use pnet::util::MacAddr;
use tokio::sync::RwLock;
use crate::state::table::{
    forwarding_table::forwarding_table::ForwardingTable,
    neighbor_table::neighbor_table::{NeighborTable, Neighbor},
    routing_table::routing_table::{RouteTable, Route},
};

use super::table::{forwarding_table::forwarding_table::{ForwadingTableKey, ForwadingTableValue}, neighbor_table::neighbor_table::{NeighborTableKey, NeighborTableValue}, routing_table::routing_table::{RoutingTableKey, RoutingTableValue}, table::TableType};


pub struct State{
    client: StateClient,
    lpm_trie: Arc<Mutex<LpmTrie<MapData, u32, [RNH;32]>>>,
    rx: Arc<RwLock<tokio::sync::mpsc::Receiver<StateCommand>>>,
}

impl State{
    pub fn new(lpm_trie: Arc<Mutex<LpmTrie<MapData, u32, [RNH;32]>>>) -> Self{
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        State{
            client: StateClient::new(tx),
            lpm_trie,
            rx: Arc::new(RwLock::new(rx)),
        }
    }
    pub fn client(&self) -> StateClient{
        self.client.clone()
    }
    pub async fn run(&self) -> anyhow::Result<Vec<tokio::task::JoinHandle<()>>>{
        info!("Starting State");
        let lt = self.lpm_trie.clone();
        let command_channel = self.rx.clone();
        let mut jh_list = Vec::new();
        let jh = tokio::spawn(async move{
            let mut command_channel = command_channel.write().await;
            let mut ft = ForwardingTable::new::<ForwadingTableKey, ForwadingTableValue>(lt.clone());
            let mut rt = RouteTable::new::<RoutingTableKey, RoutingTableValue>();
            let mut nt = NeighborTable::new::<NeighborTableKey, NeighborTableValue>();
            loop{
                while let Some(cmd) = command_channel.recv().await{
                    match cmd{
                        StateCommand::List { tx , table_type} => {
                            match table_type{
                                TableType::ForwardingTable => {
                                    let ft_list = ft.list();
                                    let mut kv_list = Vec::new();
                                    for (key, value) in &ft_list{
                                        let key: (u32,u32) = key.clone().into();
                                        let value: [RNH;32] = value.clone().into();
                                        kv_list.push(KeyValue::FORWARDING{key, value});
                                    }
                                    if let Err(_e) = tx.send(kv_list){
                                        error!("Error sending value");
                                    }
                                },
                                TableType::RouteTable => {
                                    let rt_list = rt.list();
                                    let mut kv_list = Vec::new();
                                    for (key, value) in &rt_list{
                                        let key: (u32,u8) = key.clone().into();
                                        let value: Route = value.clone().into();
                                        kv_list.push(KeyValue::ROUTE{key, value});
                                    }
                                    if let Err(_e) = tx.send(kv_list){
                                        error!("Error sending value");
                                    }
                                },
                                TableType::NeighborTable => {
                                    let nt_list = nt.list();
                                    let mut kv_list = Vec::new();
                                    for (key, value) in &nt_list{
                                        let key: u32 = key.clone().into();
                                        let value: Neighbor = value.clone().into();
                                        kv_list.push(KeyValue::NEIGHBOR{key, value});
                                    }
                                    if let Err(_e) = tx.send(kv_list){
                                        error!("Error sending value");
                                    }
                                },
                            }
                        },
                        StateCommand::Del { key } => {
                            info!("state command del");
                            match key{
                                Key::NEIGHBOR(key) => {
                                    nt.remove(key.into());
                                },
                                Key::ROUTE(prefix, prefix_len) => {
                                    rt.remove((prefix, prefix_len).into());
                                },
                                Key::FORWARDING(prefix, prefix_len) => {
                                    ft.remove((prefix, prefix_len).into());
                                },
                            }
                        }
                        StateCommand::Add(key_value) => {
                            match key_value{
                                KeyValue::NEIGHBOR{key, value} => {
                                    nt.add(key.into(), value.into());
                                },
                                KeyValue::ROUTE{key, value} => {
                                    rt.add(key.into(), value.into());
                                },
                                KeyValue::FORWARDING{key, value} => {
                                    ft.add(key.into(), value.into());
                                },
                            }
                        },
                        StateCommand::Get{key, tx} => {
                            match key{
                                Key::NEIGHBOR(key) => {
                                    let value = nt.get(key.into());
                                    match value{
                                        Some(value) => {
                                            if let Err(e) = tx.send(Some(Value::NEIGHBOR(value.into()))){
                                                error!("Error sending value: {:?}", e);
                                            }
                                        },
                                        None => {
                                            if let Err(e) = tx.send(None){
                                                error!("Error sending value: {:?}", e);
                                            }
                                        }
                                    }
                                },
                                Key::ROUTE(prefix, prefix_len) => {
                                    let value = rt.get((prefix, prefix_len).into());
                                    match value{
                                        Some(value) => {
                                            if let Err(e) = tx.send(Some(Value::ROUTE(value.into()))){
                                                error!("Error sending value: {:?}", e);
                                            }
                                        },
                                        None => {
                                            if let Err(e) = tx.send(None){
                                                error!("Error sending value: {:?}", e);
                                            }
                                        }
                                    }
                                },
                                Key::FORWARDING(prefix, prefix_len) => {
                                    let value = ft.get((prefix, prefix_len).into());
                                    match value{
                                        Some(value) => {
                                            if let Err(e) = tx.send(Some(Value::FORWARDING(value.into()))){
                                                error!("Error sending value: {:?}", e);
                                            }
                                        },
                                        None => {
                                            if let Err(e) = tx.send(None){
                                                error!("Error sending value: {:?}", e);
                                            
                                            }
                                        }
                                    }
                                },
                            }
                        },
                    
                    }
                }
            }
        });
        jh_list.push(jh);
        Ok(jh_list)
    }
}

#[derive(Debug, Clone)]
pub struct StateClient{
    tx: tokio::sync::mpsc::Sender<StateCommand>
}

impl StateClient{
    pub fn new(tx: tokio::sync::mpsc::Sender<StateCommand>) -> Self{
        StateClient{tx}
    }
    pub async fn add(&self, key_value: KeyValue) -> anyhow::Result<()>{
        self.tx.send(StateCommand::Add(key_value)).await?;
        Ok(())
    }
    pub async fn get(&self, key: Key) -> anyhow::Result<Option<Value>>{
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx.send(StateCommand::Get{key, tx}).await?;
        Ok(rx.await?)
    }
    pub async fn del(&self, key: Key) -> anyhow::Result<()>{
        self.tx.send(StateCommand::Del{key}).await?;
        Ok(())
    }

    pub async fn list(&self, table_type: TableType) -> anyhow::Result<Vec<KeyValue>>{
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx.send(StateCommand::List{tx, table_type}).await?;
        Ok(rx.await?)
    }
}

pub enum StateCommand{
    Add(KeyValue),
    Get{
        key: Key,
        tx: tokio::sync::oneshot::Sender<Option<Value>>,
    },
    Del{
        key: Key,
    },
    List{
        table_type: TableType,
        tx: tokio::sync::oneshot::Sender<Vec<KeyValue>>,
    },
}

pub enum Key{
    NEIGHBOR(u32),
    ROUTE(u32, u8),
    FORWARDING(u32, u32),
}

#[derive(Debug)]
pub enum Value{
    NEIGHBOR(Neighbor),
    ROUTE(Route),
    FORWARDING([RNH;32]),
}

impl Into<Neighbor> for Value{
    fn into(self) -> Neighbor{
        match self{
            Value::NEIGHBOR(value) => value,
            _ => panic!("Value is not a Neighbor"),
        }
    }
}

pub enum KeyValue{
    NEIGHBOR{
        key: u32,
        value: Neighbor
    },
    ROUTE{
        key: (u32, u8),
        value: Route
    },
    FORWARDING{
        key: (u32, u32),
        value: [RNH;32]
    },
}

impl Display for KeyValue{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self{
            KeyValue::NEIGHBOR{key, value} => {
                write!(f, "id: {}, neighbor: {}", *key, value)
            },
            KeyValue::ROUTE{key, value} => {
                let prefix = key.0;
                let prefix_len = key.1;
                write!(f, "{}/{} -> \n{}", Ipv4Addr::from(prefix), prefix_len, value)
            },
            KeyValue::FORWARDING{key, value} => {
                let prefix = Ipv4Addr::from(key.0);
                let prefix_len = key.1;
                let mut s = String::new();
                for rnh in value{
                    if rnh.ip == 0{
                        continue;
                    }
                    s.push_str(
                        &format!("\t\tip {}\n\t\tifidx {}\n\t\tsrc_mac {}\n\t\tdst_mac {}\n\t\ttotal_hops {}", 
                        Ipv4Addr::from(rnh.ip),
                        rnh.ifidx,
                        MacAddr::from(rnh.src_mac),
                        MacAddr::from(rnh.dst_mac),
                        rnh.total_next_hops
                    )
                    );
                }
                write!(f, "{}/{} -> \n{}", prefix, prefix_len, s)
            },
        }
    }
}