use std::{collections::HashMap, net::IpAddr, num, sync::{Arc, Mutex}};
use af_xdp::af_xdp::AfXdpClient;
use aya::maps::{LpmTrie, MapData, XskMap, HashMap as BpfHashMap};
use disco_rs::{DiscoHdr, DiscoHdrPacket, DiscoRouteHdr, DiscoRouteHdrPacket, MutableDiscoHdrPacket, MutableDiscoRouteHdrPacket};
use fabric_rs_common::{InterfaceQueue, RouteNextHop};
use interface::interface::Interface;
use log::{error, info};
use network_types::eth::EthHdr;
use pnet::{packet::{arp::ArpPacket, ethernet::{EtherType, EtherTypes, EthernetPacket, MutableEthernetPacket}, FromPacket}, util::MacAddr};
use send_receive::send_receive::{SendReceive, SendReceiveClient};
use state::{
    state::{Key, KeyValue, State, StateClient},
        table::{neighbor_table::neighbor_table::{Neighbor, NeighborInterface},
        routing_table::routing_table::{Route, RouteNextHop as RNH, RouteNextHopList, RouteOrigin}
}};
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
    pub async fn run(&mut self, xsk_map: XskMap<MapData>, interface_queue_table: BpfHashMap<MapData, InterfaceQueue, u32>, send: Option<bool>) -> anyhow::Result<()>{
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

        local_routes(self.interfaces.clone(),  state_client.clone(), self.id).await?;
        
        let jh = af_xdp.run(recv_tx);
        jh_list.extend(jh.await?);
        info!("{} af_xdp tasks running", jh_list.len());
        
        let jh = self.receiver(recv_rx, af_xdp_client.clone(), state_client.clone());
        jh_list.push(jh.await?);
        info!("{} receiver tasks running", jh_list.len());
        
        let send = if let Some(send) = send{
            send
        } else {
            true
        };
        if send{
            let jh = self.disco_sender( af_xdp_client);
            jh_list.push(jh.await?);
            info!("{} disco sender tasks running", jh_list.len());
        }
        
        let cli = Cli::new();
        let jh = cli.run(state_client);
        jh_list.extend(jh.await?);
        info!("{} cli tasks running", jh_list.len());

        info!("running {} tasks", jh_list.len());

        futures::future::join_all(jh_list).await;

        Ok(())
    }

    pub async fn receiver(&self, mut rx: tokio::sync::mpsc::Receiver<(u32, Vec<u8>)>, af_xdp_client: AfXdpClient, state_client: StateClient) -> anyhow::Result<tokio::task::JoinHandle<()>>{
        let id = self.id;
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
                            if let Err(e) = disco_handler(eth_packet, interface, state_client.clone(), af_xdp_client.clone(), id).await{
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

pub async fn local_routes(interface_list: HashMap<u32, Interface>, state_client: StateClient, id: u32) -> anyhow::Result<()>{
    for (_ifidx, interface) in &interface_list{
        let mut route_next_hop_list = RouteNextHopList::new();
        let route_next_hop = RNH::new(
            id,
            interface.ip.into(),
            0,
            interface.ifidx,
            interface.mac.into(),
            interface.mac.into(),
        );
        route_next_hop_list.add(route_next_hop);
        let route = Route::new(
            RouteOrigin::LOCAL,
            route_next_hop_list,
        );
        let route_value = KeyValue::ROUTE { key: (interface.ip.into(), 32), value: route };
        state_client.add(route_value).await?;
    }
    Ok(())
}

pub async fn disco_handler(eth_packet: EthernetPacket<'_>, interface: Interface, state_client: StateClient, mut af_xdp_client: AfXdpClient, id: u32) -> anyhow::Result<()>{
    let disco_packet_hdr = if let Some(disco_packet_hdr) = DiscoHdrPacket::new(eth_packet.payload()[..DiscoHdr::LEN].as_ref()){
        disco_packet_hdr
    } else {
        return Err(anyhow::anyhow!("Error parsing disco packet"));
    };
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

        let local_routes = state_client.list(state::table::table::TableType::RouteTable).await?;
        let mut routes = Vec::new();
        let mut route_count = 0;
        for local_route in &local_routes{
            let new_disco_route_packet_hdr = match local_route{
                KeyValue::ROUTE { key: (ip,prefix_len), value: route } => {
                    let mut from_neighbor = false;
                    for rnh in route.get_next_hops(){
                        if disco_packet_hdr.get_id() == rnh.originator_id{
                            from_neighbor = true;
                        }
                    }
                    if !from_neighbor{
                        let mut disco_route_hdr_buffer = [0u8; DiscoRouteHdr::LEN];
                        let mut disco_route_hdr_packet = MutableDiscoRouteHdrPacket::new(&mut disco_route_hdr_buffer).unwrap();
                        disco_route_hdr_packet.set_ip(*ip);
                        disco_route_hdr_packet.set_prefix_len(*prefix_len as u32);
                        disco_route_hdr_packet.set_hops(3);
                        Some(disco_route_hdr_packet.packet().to_vec())
                    } else {
                        None
                    }
                }
                _ => {None}
            };
            if let Some(new_disco_packet_hdr) = new_disco_route_packet_hdr{
                routes.extend(new_disco_packet_hdr);
                route_count += 1;
            }
        }
        if route_count > 0 {

            let mut disco_packet_buffer = [0u8; DiscoHdr::LEN];
            let mut disco_packet = MutableDiscoHdrPacket::new(&mut disco_packet_buffer).unwrap();
            disco_packet.set_id(id);
            disco_packet.set_ip(interface.ip.into());
            disco_packet.set_op(1);
            disco_packet.set_len(route_count);
            let mut disco_packet = disco_packet.packet().to_vec();
            disco_packet.extend(routes.clone());



            let mut ethernet_packet_buffer = [0u8; EthHdr::LEN];
            let mut ethernet_packet = MutableEthernetPacket::new(&mut ethernet_packet_buffer).unwrap();
            ethernet_packet.set_destination(eth_packet.get_source());
            ethernet_packet.set_source(interface.mac.into());
            ethernet_packet.set_ethertype(EtherType(0x0060));

            let mut ethernet_packet = ethernet_packet.packet().to_vec();
            ethernet_packet.extend(disco_packet);

            af_xdp_client.send(interface.ifidx, ethernet_packet).await?;
        }

    }
    if disco_packet_hdr.get_op() == 1 {
        //info!("DISCOROUTEPACKET: {}, pl: {}", disco_packet_hdr,eth_packet.payload().len() );
        let disco_route_packet = &eth_packet.payload()[DiscoHdr::LEN..];
        let number_of_route_headers = disco_packet_hdr.get_len();
        for i in 0..number_of_route_headers{
            /*
            let disco_route_hdr_packet = if let Some(disco_route_hdr_packet) = DiscoRouteHdrPacket::new(&disco_route_packet[(i*DiscoRouteHdr::LEN) as usize..(i*DiscoRouteHdr::LEN + DiscoRouteHdr::LEN) as usize]){
                disco_route_hdr_packet
            } else {
                continue;
            };
            info!("Disco route header: {}", disco_route_hdr_packet);
            */
            /*
            let ip = disco_route_hdr_packet.get_ip();
            let prefix_len = disco_route_hdr_packet.get_prefix_len();
            let hops = disco_route_hdr_packet.get_hops();
            let route_next_hop = RNH::new(
                disco_packet_hdr.get_id(),
                ip,
                hops,
                interface.ifidx,
                eth_packet.get_source().into(),
                disco_packet_hdr.get_mac().into(),
            );
            let route_next_hop_list = RouteNextHopList::new();
            route_next_hop_list.add(route_next_hop);
            let route = Route::new(
                RouteOrigin::REMOTE,
                route_next_hop_list,
            );
            let route_value = KeyValue::ROUTE { key: (ip, prefix_len), value: route };
            state_client.add(route_value).await?;
            */
        }

    }
    Ok(())
}

pub async fn arp_handler(arp_packet: ArpPacket<'_>, ifidx: u32, state_client: StateClient, sr_client: SendReceiveClient) -> anyhow::Result<()>{
    Ok(())
}



