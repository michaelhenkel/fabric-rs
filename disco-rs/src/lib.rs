use std::{mem, fmt::{self, Display}};
use pnet_macros::Packet;
use log::info;
use pnet_macros_support::{types::{u16be, u32be}, packet::Packet};
use pnet_base::MacAddr;

#[derive(Clone, Packet)]
pub struct DiscoHdr{
    pub id: u32be,
    pub ip: u32be,
    pub len: u16be,
    #[construct_with(u8, u8, u8, u8, u8, u8)]
    pub mac: MacAddr,
    pub op: u8,
    pub res_1: u8,
    pub res_2: u8,
    pub res_3: u8,
    #[payload]
    pub payload: Vec<u8>
}

impl DiscoHdr {
    pub const LEN: usize = 20;
}

impl DiscoHdrPacket<'_>{
    pub const LEN: usize = mem::size_of::<DiscoHdrPacket>();

}

impl Display for DiscoHdrPacket<'_>{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = self.get_id();
        let ip = std::net::Ipv4Addr::from(self.get_ip());
        let op = self.get_op();
        let len = self.get_len();
        let mut route_hdr_str = "".to_string();
        for i in 0..len as usize{
            let offset = i * DiscoRouteHdr::LEN;            
            let pl = &self.payload()[offset..offset + DiscoRouteHdr::LEN];
            let disco_route_header: &[u8;DiscoRouteHdr::LEN] = &pl.try_into().unwrap();
            let route_hdr = DiscoRouteHdrPacket::new(disco_route_header).unwrap();
            route_hdr_str = format!("{}\n{}", route_hdr_str, route_hdr);
            
        }
        write!(f, "id: {}, ip: {}, op: {}, len:{},  rh:\n{}", id, ip, op, len, route_hdr_str)
    }
}

#[derive(Clone, Packet)]
pub struct DiscoRouteHdr{
    pub ip: u32be,
    pub hops: u32be,
    pub prefix_len: u32be,
    #[payload]
    pub payload: Vec<u8>
}

#[derive(Clone, Packet)]
pub struct DiscoRouteNextHopHdr{
    pub nh_ip: u32be,
    #[payload]
    pub payload: Vec<u8>
}

impl Display for MutableDiscoHdrPacket<'_>{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = self.get_id();
        let ip = std::net::Ipv4Addr::from(self.get_ip());
        let op = self.get_op();
        let number_of_route_hdrs: usize = self.get_len() as usize;
        let mut route_hdr_str = "".to_string();
        for i in 0..number_of_route_hdrs{
            let offset = i * DiscoRouteHdr::LEN;
            let pl = &self.payload()[offset..offset + DiscoRouteHdr::LEN];
            let disco_route_header: &[u8;DiscoRouteHdr::LEN] = &pl.try_into().unwrap();
            let route_hdr = DiscoRouteHdrPacket::new(disco_route_header).unwrap();
            route_hdr_str = format!("{}\n{}", route_hdr_str, route_hdr);
        }        
        write!(f, "id: {}, ip: {}, op: {}, rh:\n{}", id, ip, op, route_hdr_str)
    }

}

impl DiscoRouteHdr{
    pub const LEN: usize = 12;
}

impl DiscoRouteHdrPacket<'_>{
    pub const LEN: usize = mem::size_of::<DiscoRouteHdrPacket>();
}

impl Display for MutableDiscoRouteHdrPacket<'_>{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ip = std::net::Ipv4Addr::from(self.get_ip());
        let hops: u32 = self.get_hops();
        let prefix_len = self.get_prefix_len();
        write!(f, "ip: {}, hops: {}, prefix_len: {}", ip, hops, prefix_len)
    }
}

impl Display for DiscoRouteHdrPacket<'_>{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ip = std::net::Ipv4Addr::from(self.get_ip());
        let hops: u32 = self.get_hops();
        let prefix_len = self.get_prefix_len();
        write!(f, "ip: {}, hops: {}, prefix_len: {}", ip, hops, prefix_len)
    }
}