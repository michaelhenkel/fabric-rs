use std::{collections::HashMap, net::IpAddr, sync::{Arc, Mutex}};
use af_xdp::af_xdp::AfXdpClient;
use aya::maps::{LpmTrie, MapData, XskMap, HashMap as BpfHashMap};
use disco_rs::{DiscoHdr, DiscoHdrPacket, MutableDiscoHdrPacket};
use fabric_rs_common::{InterfaceQueue, RouteNextHop};
use interface::interface::Interface;
use log::{error, info};
use network_types::eth::EthHdr;
use pnet::{packet::{arp::ArpPacket, ethernet::{EtherType, EtherTypes, EthernetPacket, MutableEthernetPacket}}, util::MacAddr};
use send_receive::send_receive::{SendReceive, SendReceiveClient};
use state::{state::{Key, KeyValue, State, StateClient}, table::neighbor_table::neighbor_table::{Neighbor, NeighborInterface}};
use cli::cli::Cli;
use pnet::packet::Packet;
use crate::af_xdp::af_xdp::AfXdp;
use fabric_rs_common::DiscoHdr as DiscoHdr2;

pub mod interface;
pub mod send_receive;
pub mod state;
pub mod cli;
pub mod af_xdp;

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
    pub async fn run(&mut self, xsk_map: XskMap<MapData>, interface_queue_table: BpfHashMap<MapData, InterfaceQueue, u32>) -> anyhow::Result<()>{
        let (recv_tx, recv_rx) = tokio::sync::mpsc::channel(100);

        let state = State::new(self.lpm_trie.clone());
        let state_client = state.client();
        self.state_client = Some(state_client.clone());

        let mut af_xdp = AfXdp::new(self.interfaces.clone(), xsk_map, interface_queue_table);
        let af_xdp_client = af_xdp.client();

        let mut jh_list = Vec::new();

        let jh = state.run();
        jh_list.extend(jh.await?);
        info!("{} state tasks running", jh_list.len());
        
        let jh = af_xdp.run(recv_tx);
        jh_list.extend(jh.await?);
        info!("{} af_xdp tasks running", jh_list.len());
        
        let jh = self.receiver(recv_rx, af_xdp_client.clone(), state_client.clone());
        jh_list.push(jh.await?);
        info!("{} receiver tasks running", jh_list.len());
        
        let jh = self.disco_sender( af_xdp_client);
        jh_list.push(jh.await?);
        info!("{} disco sender tasks running", jh_list.len());
        
        let cli = Cli::new();
        let jh = cli.run(state_client);
        jh_list.extend(jh.await?);
        info!("{} cli tasks running", jh_list.len());

        info!("running {} tasks", jh_list.len());

        futures::future::join_all(jh_list).await;

        Ok(())
    }

    pub async fn receiver(&self, mut rx: tokio::sync::mpsc::Receiver<(u32, Vec<u8>)>, af_xdp_client: AfXdpClient, state_client: StateClient) -> anyhow::Result<tokio::task::JoinHandle<()>>{
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
                            //if let Err(e) = arp_handler(arp_packet, ifidx, state_client.clone(), sr_client.clone()).await{
                            //    error!("Error handling arp: {:?}", e);
                            //}
                        },
                        EtherType(0x0060) => {
                            let interface = if let Some(interface) = interfaces.get(&ifidx){
                                interface.clone()
                            } else {
                                continue;
                            };
                            if let Err(e) = disco_handler(eth_packet, interface, state_client.clone()).await{
                                error!("Error handling disco: {:?}", e);
                            }
                        },
                        _ => {
                            continue;
                        },
                    }
                }
            }
        });
        Ok(jh)
    }

    pub async fn disco_sender(&self, mut afxdp_client: AfXdpClient) -> anyhow::Result<tokio::task::JoinHandle<()>>{
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
                    disco_packet.set_len(0);
                    ethernet_packet.set_payload(disco_packet.packet());
                    if let Err(e) = afxdp_client.send(interface.ifidx, ethernet_packet.packet().to_vec()).await{
                        println!("Error sending disco: {:?}", e);
                    }
                }
            }
        });
        Ok(jh)
    }
}


pub async fn disco_handler(eth_packet: EthernetPacket<'_>, interface: Interface, state_client: StateClient) -> anyhow::Result<()>{
    info!("eth payload len: {}, offset: {} end:", eth_packet.payload().len(), EthHdr::LEN, );

    let disco_packet_hdr = if let Some(disco_packet_hdr) = DiscoHdrPacket::new(eth_packet.payload()[..DiscoHdr::LEN].as_ref()){
        disco_packet_hdr
    } else {
        return Err(anyhow::anyhow!("Error parsing disco packet"));
    };
    info!("Disco packet: {}", disco_packet_hdr);
    if disco_packet_hdr.get_op() == 0 {
        
        let neighbor_interface = NeighborInterface::new(
            disco_packet_hdr.get_ip(),
            eth_packet.get_source().into(),
            interface.ip.into(),
            interface.mac.into(),
            interface.ifidx,
        );
        match state_client.get(Key::NEIGHBOR(disco_packet_hdr.get_id())).await{
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
                state_client.add(KeyValue::NEIGHBOR { key: disco_packet_hdr.get_id(), value: neighbor }).await?;
            },
            Err(e) => {
                return Err(e)
            }
        }
    }
    if disco_packet_hdr.get_op() == 1 {

    }
    Ok(())
}

pub async fn arp_handler(arp_packet: ArpPacket<'_>, ifidx: u32, state_client: StateClient, sr_client: SendReceiveClient) -> anyhow::Result<()>{
    Ok(())
}



