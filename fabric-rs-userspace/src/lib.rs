use std::{collections::HashMap, sync::{Arc, Mutex}};
use aya::maps::{LpmTrie, MapData};
use disco_rs::{DiscoHdr, DiscoHdrPacket, MutableDiscoHdrPacket};
use fabric_rs_common::RouteNextHop;
use interface::interface::Interface;
use log::error;
use network_types::eth::EthHdr;
use pnet::{packet::{arp::ArpPacket, ethernet::{EtherType, EtherTypes, EthernetPacket, MutableEthernetPacket}}, util::MacAddr};
use send_receive::send_receive::{SendReceive, SendReceiveClient};
use state::{state::{Key, KeyValue, State, StateClient}, table::neighbor_table::neighbor_table::{Neighbor, NeighborInterface}};
use cli::cli::Cli;
use pnet::packet::Packet;

pub mod interface;
pub mod send_receive;
pub mod state;
pub mod cli;

pub struct UserSpace{
    lpm_trie: Arc<Mutex<LpmTrie<MapData, u32, [RouteNextHop;32]>>>,
    interfaces: HashMap<u32,Interface>,
    id: u32,
    sr_client: Option<SendReceiveClient>,
    state_client: Option<StateClient>,
}

impl UserSpace{
    pub fn new(
        interfaces: HashMap<u32,Interface>,
        id: u32,
        lpm_trie: LpmTrie<MapData, u32, [RouteNextHop;32]>
    ) -> Self{
        UserSpace{
            lpm_trie: Arc::new(Mutex::new(lpm_trie)),
            interfaces,
            id,
            sr_client: None,
            state_client: None,
        }
    }
    pub async fn run(&mut self) -> anyhow::Result<()>{
        let (recv_tx, recv_rx) = tokio::sync::mpsc::channel(100);
        let mut sr = SendReceive::new(self.interfaces.clone(), recv_tx.clone());
        let sr_client = sr.client();
        self.sr_client = Some(sr_client.clone());
        let state = State::new(self.lpm_trie.clone());
        let state_client = state.client();
        self.state_client = Some(state_client.clone());

        let mut jh_list = Vec::new();

        let jh = sr.run();
        jh_list.extend(jh.await?);

        let jh = state.run();
        jh_list.extend(jh.await?);

        let jh = self.receiver(recv_rx);
        jh_list.push(jh.await?);

        let jh = self.disco_sender(sr_client);
        jh_list.push(jh.await?);

        let cli = Cli::new();

        let jh = cli.run(state_client);
        jh_list.extend(jh.await?);

        futures::future::join_all(jh_list).await;

        Ok(())
    }

    pub async fn receiver(&self, mut rx: tokio::sync::mpsc::Receiver<(u32, Vec<u8>)>) -> anyhow::Result<tokio::task::JoinHandle<()>>{
        let sr_client = if let Some(sr_client) = &self.sr_client{
            sr_client.clone()
        } else {
            return Err(anyhow::anyhow!("sr_client not found"));
        };
        let state_client = if let Some(state_client) = &self.state_client{
            state_client.clone()
        } else {
            return Err(anyhow::anyhow!("state_client not found"));
        };
        let interfaces = self.interfaces.clone();
        let jh = tokio::spawn(async move{
            loop{
                while let Some((ifidx, msg)) = rx.recv().await{
                    let eth_packet = if let Some(eth_packet) = EthernetPacket::new(msg.as_slice()){
                        eth_packet
                    } else {
                        continue;
                    };
                    match eth_packet.get_ethertype(){
                        EtherTypes::Arp => {
                            let arp_packet = if let Some(arp_packet) = ArpPacket::new(eth_packet.payload()){
                                arp_packet
                            } else {
                                continue;
                            };
                            if let Err(e) = arp_handler(arp_packet, ifidx, state_client.clone(), sr_client.clone()).await{
                                error!("Error handling arp: {:?}", e);
                            }
                        },
                        EtherType(0x0060) => {
                            let disco_packet = if let Some(disco_packet) = DiscoHdrPacket::new(eth_packet.payload()){
                                disco_packet
                            } else {
                                continue;
                            };
                            let interface = if let Some(interface) = interfaces.get(&ifidx){
                                interface.clone()
                            } else {
                                continue;
                            };
                            if let Err(e) = disco_handler(disco_packet, interface, state_client.clone(), sr_client.clone()).await{
                                error!("Error handling disco: {:?}", e);
                            }
                        },
                        _ => {
                            continue;
                        },
                    }
                    println!("Received: {:?}", msg);
                }
            }
        });
        Ok(jh)
    }

    pub async fn disco_sender(&self, sr_client: SendReceiveClient) -> anyhow::Result<tokio::task::JoinHandle<()>>{
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        let interfaces = self.interfaces.clone();
        let id = self.id.clone();
        let jh = tokio::spawn(async move{
            loop{
                interval.tick().await;
                for (_, interface) in interfaces.iter(){
                    let mut ethernet_buffer = [0u8; EthHdr::LEN + 24];
                    let mut ethernet_packet = MutableEthernetPacket::new(&mut ethernet_buffer).unwrap();
                    ethernet_packet.set_destination(MacAddr::broadcast());
                    ethernet_packet.set_source(interface.mac.into());
                    ethernet_packet.set_ethertype(EtherType(0x0060));
                    let mut disco_buffer = [0u8; 24];
                    let mut disco_packet = MutableDiscoHdrPacket::new(&mut disco_buffer).unwrap();
                    disco_packet.set_id(id);
                    disco_packet.set_ip(u32::from_be_bytes(interface.ip.octets()));
                    disco_packet.set_op(0);
                    ethernet_packet.set_payload(disco_packet.packet());
                    if let Err(e) = sr_client.send(interface.ifidx, ethernet_packet.packet().to_vec()).await{
                        println!("Error sending disco: {:?}", e);
                    }
                }
            }
        });
        Ok(jh)
    }
}


pub async fn disco_handler(disco_packet: DiscoHdrPacket<'_>, interface: Interface, state_client: StateClient, sr_client: SendReceiveClient) -> anyhow::Result<()>{
    if disco_packet.get_op() == 0 {
        let neighbor_interface = NeighborInterface::new(
            disco_packet.get_ip(),
            disco_packet.get_mac().into(),
            interface.ip.into(),
            interface.mac.into(),
            interface.ifidx,
        );
        match state_client.get(Key::NEIGHBOR(disco_packet.get_id())).await{
            Ok(neighbor) => {
                let neighbor = match neighbor {
                    Some(neighbor) => {
                        let mut neighbor: Neighbor = neighbor.into();
                        neighbor.add_interface(neighbor_interface);
                        neighbor
                        
                    },
                    None => {
                        let mut neighbor = Neighbor::new();
                        neighbor.add_interface(neighbor_interface);
                        neighbor
                    }
                };
                state_client.add(KeyValue::NEIGHBOR { key: disco_packet.get_id(), value: neighbor }).await?;
            },
            Err(e) => {
                return Err(e)
            }
        }
    }
    if disco_packet.get_op() == 1 {

    }
    Ok(())
}

pub async fn arp_handler(arp_packet: ArpPacket<'_>, ifidx: u32, state_client: StateClient, sr_client: SendReceiveClient) -> anyhow::Result<()>{
    Ok(())
}



