#![no_std]

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