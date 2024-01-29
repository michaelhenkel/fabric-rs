#![no_std]
use core::mem;

#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
pub struct InterfaceConfig{
    pub instance_type: u8,
}
#[cfg(feature = "user")]
unsafe impl aya::Pod for InterfaceConfig {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FlowNextHop {
    pub src_mac: [u8;6],
    pub dst_mac: [u8;6],
    pub src_ip: u32,
    pub dst_ip: u32,
    pub ifidx: u32,
    pub flowlet_size: u32,
    pub counter: u32,
    pub current_link: u32
}

#[cfg(feature = "user")]
unsafe impl aya::Pod for FlowNextHop {}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct RouteNextHop {
    pub ip: u32,
    pub ifidx: u32,
    pub src_mac: [u8;6],
    pub dst_mac: [u8;6],
    pub total_next_hops: u32,
}
#[cfg(feature = "user")]
unsafe impl aya::Pod for RouteNextHop {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InterfaceQueue {
    pub ifidx: u32,
    pub queue: u32,
}

impl InterfaceQueue {
    pub fn new(ifidx: u32, queue: u32) -> Self {
        InterfaceQueue { ifidx, queue }
    }
}

#[cfg(feature = "user")]
unsafe impl aya::Pod for InterfaceQueue {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DiscoHdr{
    pub id: u32,
    pub ip: u32,
    pub len: u16,
    pub mac: [u8;6],
    pub op: u8,
    pub res_1: u8,
    pub res_2: u8,
    pub res_3: u8,
}

impl DiscoHdr{
    pub const LEN: usize = mem::size_of::<DiscoHdr>();
}

#[derive(Clone, Copy)]
pub struct DiscoRouteHdr{
    pub ip: u32,
    pub hops: u32,
    pub prefix_len: u32,
}

impl DiscoRouteHdr{
    pub const LEN: usize = mem::size_of::<DiscoRouteHdr>();
}