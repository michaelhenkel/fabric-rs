use std::fmt::{self, Display};

pub trait Table<K: TableKey,V: TableValue>: Send {
    fn add(&mut self, k: K, v: V);
    fn get(&self, k: K) -> Option<V>;
    fn remove(&mut self, k: K);
    fn list(&self) -> Vec<(K,V)>;
}

pub trait TableKey{}

pub trait TableValue{}

pub enum TableType{
    NeighborTable,
    RouteTable,
    ForwardingTable,
}

impl Display for TableType{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TableType::NeighborTable => write!(f, "NeighborTable"),
            TableType::RouteTable => write!(f, "RouteTable"),
            TableType::ForwardingTable => write!(f, "ForwardingTable"),
        }
    }
}