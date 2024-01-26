#![no_std]
#![no_main]

use core::{mem, f32::consts::E};
use network_types::{
    eth::{EthHdr, EtherType},
    ip::{Ipv4Hdr, IpProto},
    udp::UdpHdr,
};
use aya_bpf::{
    bindings::xdp_action,
    macros::{xdp,map},
    programs::{XdpContext, xdp},
    maps::{HashMap, lpm_trie::{LpmTrie, Key}, XskMap}, helpers::bpf_redirect,
};
use aya_log_ebpf::{info, warn};
use fabric_rs_common::{InterfaceConfig, RouteNextHop};

#[map(name = "INTERFACECONFIG")]
static mut INTERFACECONFIG: HashMap<u32, InterfaceConfig> =
    HashMap::<u32, InterfaceConfig>::with_max_entries(128, 0);

#[map(name = "FORWARDINGTABLE")]
static mut FORWARDINGTABLE: HashMap<u32, RouteNextHop> =
    HashMap::<u32, RouteNextHop>::with_max_entries(2048, 0);

#[map(name = "ROUTINGTABLE")]
static mut ROUTINGTABLE: LpmTrie<u32, [RouteNextHop;32]> =
    LpmTrie::<u32, [RouteNextHop;32]>::with_max_entries(2048, 0);

#[map(name = "XSKMAP")]
static mut XSKMAP: XskMap = XskMap::with_max_entries(8, 0);


pub enum InstanceType {
    INSTANCE,
    NETWORK,
}

#[xdp]
pub fn fabric_rs(ctx: XdpContext) -> u32 {
    match try_fabric_rs(ctx) {
        Ok(ret) => ret,
        Err(_) => xdp_action::XDP_ABORTED,
    }
}

fn try_fabric_rs(ctx: XdpContext) -> Result<u32, u32> {
    
    let ingress_if_idx = unsafe { (*ctx.ctx).ingress_ifindex };
    let queue_idx = unsafe { (*ctx.ctx).rx_queue_index };
    info!(
        &ctx,
        "ingress_if_idx: {}, queue_idx: {}", ingress_if_idx, queue_idx
    );

    let eth_hdr = ptr_at_mut::<EthHdr>(&ctx, 0).ok_or(xdp_action::XDP_ABORTED)?;
    
    if unsafe { (*eth_hdr).ether_type } == EtherType::Loop {
        info!(&ctx, "disco packet received");

        if let Some(fd) = unsafe { XSKMAP.get(queue_idx)}{
            info!(&ctx, "xsk_map.get returned fd: {}", fd);
        } else {
            info!(&ctx, "xsk_map.get returned None");
        }

        match unsafe{ XSKMAP.redirect(queue_idx, 0) }{
            Ok(res) => {
                info!(&ctx, "bpf_redirect returned: {}", res);
                return Ok(res)
            },
            Err(e) => {
                info!(&ctx, "bpf_redirect returned error: {}", e);
                return Ok(xdp_action::XDP_PASS);
            }
        }
    }



    if unsafe { (*eth_hdr).ether_type } == EtherType::Arp {
        info!(&ctx, "arp packet received");

        if let Some(fd) = unsafe { XSKMAP.get(queue_idx)}{
            info!(&ctx, "xsk_map.get returned fd: {}", fd);
        } else {
            info!(&ctx, "xsk_map.get returned None");
        }

        match unsafe{ XSKMAP.redirect(queue_idx, 0) }{
            Ok(res) => {
                info!(&ctx, "bpf_redirect returned: {}", res);
                return Ok(res)
            },
            Err(e) => {
                info!(&ctx, "bpf_redirect returned error: {}", e);
                return Ok(xdp_action::XDP_PASS);
            }
        }
    }

    let ipv4_hdr = ptr_at::<Ipv4Hdr>(&ctx, EthHdr::LEN)
        .ok_or(xdp_action::XDP_ABORTED)?;

    if unsafe { (*ipv4_hdr).proto } == IpProto::Icmp {
        info!(&ctx, "icmp packet received");

        if let Some(fd) = unsafe { XSKMAP.get(queue_idx)}{
            info!(&ctx, "xsk_map.get returned fd: {}", fd);
        } else {
            info!(&ctx, "xsk_map.get returned None");
        }

        match unsafe{ XSKMAP.redirect(queue_idx, 0) }{
            Ok(res) => {
                info!(&ctx, "bpf_redirect returned: {}", res);
                return Ok(res)
            },
            Err(e) => {
                info!(&ctx, "bpf_redirect returned error: {}", e);
                return Ok(xdp_action::XDP_PASS);
            }
        }
    }

    let dst_ip = unsafe { (*ipv4_hdr).dst_addr };
    let src_ip = unsafe { (*ipv4_hdr).src_addr };

    let key = Key::new(32, u32::from_be(dst_ip));

    if let Some (route_next_hops) = unsafe { ROUTINGTABLE.get(&key)}{
        info!(&ctx, "found route_next_hops for: {:i}", u32::from_be(dst_ip));

        let mut total_next_hops = 0;
        for nh in route_next_hops{
            total_next_hops = nh.total_next_hops;
            break;
            
        }
        if total_next_hops > 0 {
            //info!(&ctx, "found route_next_hop: {}", total_next_hops);
            if route_next_hops.len() > 0 {
                let nh = route_next_hops[0];
                unsafe {
                    (*eth_hdr).src_addr[0] = nh.dst_mac[0];
                    (*eth_hdr).src_addr[1] = nh.dst_mac[1];
                    (*eth_hdr).src_addr[2] = nh.dst_mac[2];
                    (*eth_hdr).src_addr[3] = nh.dst_mac[3];
                    (*eth_hdr).src_addr[4] = nh.dst_mac[4];
                    (*eth_hdr).src_addr[5] = nh.dst_mac[5];
                    (*eth_hdr).dst_addr[0] = nh.src_mac[0];
                    (*eth_hdr).dst_addr[1] = nh.src_mac[1];
                    (*eth_hdr).dst_addr[2] = nh.src_mac[2];
                    (*eth_hdr).dst_addr[3] = nh.src_mac[3];
                    (*eth_hdr).dst_addr[4] = nh.src_mac[4];
                    (*eth_hdr).dst_addr[5] = nh.src_mac[5];
                }
                info!(
                    &ctx,
                    "\nsrc_mac: {:x}:{:x}:{:x}:{:x}:{:x}:{:x}\ndst_mac: {:x}:{:x}:{:x}:{:x}:{:x}:{:x}\nifidx: {}\nsrc_ip: {:i}\ndst_ip: {:i}",
                    unsafe { (*eth_hdr).src_addr[0] },
                    unsafe { (*eth_hdr).src_addr[1] },
                    unsafe { (*eth_hdr).src_addr[2] },
                    unsafe { (*eth_hdr).src_addr[3] },
                    unsafe { (*eth_hdr).src_addr[4] },
                    unsafe { (*eth_hdr).src_addr[5] },
                    unsafe { (*eth_hdr).dst_addr[0] },
                    unsafe { (*eth_hdr).dst_addr[1] },
                    unsafe { (*eth_hdr).dst_addr[2] },
                    unsafe { (*eth_hdr).dst_addr[3] },
                    unsafe { (*eth_hdr).dst_addr[4] },
                    unsafe { (*eth_hdr).dst_addr[5] },
                    nh.ifidx,
                    u32::from_be(src_ip),
                    u32::from_be(dst_ip)
                );
                let res = unsafe { bpf_redirect(nh.ifidx, 0)};
                info!(&ctx, "bpf_redirect returned: {}", res);
                return Ok(res as u32)
            }
        } else {
            info!(&ctx, "no route_next_hop found for dst_ip: {:i}", u32::from_be(dst_ip));
        }
    } else {
        info!(&ctx, "no route_next_hop found for dst_ip: {:i}", u32::from_be(dst_ip));
    }


    return Ok(xdp_action::XDP_PASS);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}

#[inline(always)]
pub fn ptr_at_mut<T>(ctx: &XdpContext, offset: usize) -> Option<*mut T> {
    let ptr = ptr_at::<T>(ctx, offset)?;
    Some(ptr as *mut T)
}

#[inline(always)]
pub fn ptr_at<T>(ctx: &XdpContext, offset: usize) -> Option<*const T> {
    let start = ctx.data();
    let end = ctx.data_end();
    let len = mem::size_of::<T>();

    if start + offset + len > end {
        return None;
    }

    Some((start + offset) as *const T)
}
