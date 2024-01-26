use core::{mem::MaybeUninit, num::NonZeroU32, ptr::NonNull};
use std::{collections::HashMap, sync::{Arc, Mutex}};
use pnet::packet::ethernet::EthernetPacket;
use tokio::sync::RwLock;
use xdpilone::xdp::XdpDesc;
use aya::maps::{MapData, XskMap};
use xdpilone::{BufIdx, IfInfo, Socket, SocketConfig, Umem, UmemConfig};
use log::info;
use crate::interface::interface::Interface;

#[repr(align(4096))]
struct PacketMap(MaybeUninit<[u8; 1 << 20]>);
pub struct AfXdp{
    client: AfXdpClient,
    interface_list: HashMap<u32, Interface>,
    xsk_map: Arc<Mutex<XskMap<MapData>>>,
}

impl AfXdp{
    pub fn new(interface_list: HashMap<u32, Interface>, xsk_map: XskMap<MapData>) -> Self{
        AfXdp{
            client: AfXdpClient::new(),
            interface_list,
            xsk_map: Arc::new(Mutex::new(xsk_map)),
        }
    }
    pub fn client(&self) -> AfXdpClient{
        self.client.clone()
    }
    //pub async fn run(&mut self) -> anyhow::Result<Vec<tokio::task::JoinHandle<()>>>{
    pub async fn run(&mut self) -> anyhow::Result<()>{
        let alloc = Box::new(PacketMap(MaybeUninit::uninit()));
        let mem = NonNull::new(Box::leak(alloc).0.as_mut_ptr()).unwrap();
        let umem = unsafe { Umem::new(UmemConfig::default(), mem) }.unwrap();
        //let mut jh_list = Vec::new();
        for (ifidx, interface) in &self.interface_list{
            let xsk_map = self.xsk_map.clone();
            let info = ifinfo(&interface.name, Some(0)).unwrap();
            let sock = Socket::with_shared(&info, &umem).unwrap();
            let mut device_queue = umem.fq_cq(&sock).unwrap();
            let rxtx = umem
            .rx_tx(
                &sock,
                &SocketConfig {
                    rx_size: NonZeroU32::new(32),
                    tx_size: NonZeroU32::new(1 << 14),
                    bind_flags: SocketConfig::XDP_BIND_NEED_WAKEUP,
                    //bind_flags: SocketConfig::XDP_BIND_ZEROCOPY | SocketConfig::XDP_BIND_NEED_WAKEUP,
                },
            ).unwrap();

            let tx = match rxtx.map_tx(){
                Ok(tx) => tx,
                Err(e) => {
                    info!("Error mapping tx: {}", e);
                    panic!("Error mapping tx");
                },
            
            };
            let rx = match rxtx.map_rx(){
                Ok(rx) => rx,
                Err(e) => {
                    info!("Error mapping rx: {}", e);
                    panic!("Error mapping rx");
                },
            
            };
            umem.bind(&rxtx).unwrap();

            let mut xsk_map = xsk_map.lock().unwrap();
            xsk_map.set(0, device_queue.as_raw_fd(), 0).unwrap();
            
            let mut tx = tx;
            let mut rx = rx;

            //let mut frames = Vec::new();

            //{
                /*  
                for i in 0..10{
                    let frame = umem.frame(BufIdx(i)).unwrap();
                    frames.push(frame.offset);
                }
                */
            
                let frame = umem.frame(BufIdx(0)).unwrap();
            {
                let mut writer = device_queue.fill(1);
                writer.insert_once(frame.offset);
                //let x = writer.insert(frames.into_iter());
                //info!("Inserted {} frames", x);
                writer.commit();
        
            }
            info!("done reading");
            
            loop{
                let mut receive = rx.receive(1);
                if let Some(desc) = receive.read(){
                    
                    let buf = unsafe {
                        &frame.addr.as_ref()[desc.addr as usize..(desc.addr as usize + desc.len as usize)]
                    };
                    let (eth, buf) = buf.split_at(14);
                    let eth_packet = EthernetPacket::new(eth).unwrap();
                    info!("Received packet: {:?}", eth_packet);
                    receive.release();
                    let frame = umem.frame(BufIdx(0)).unwrap();
                    {
                        let mut writer = device_queue.fill(1);
                        writer.insert_once(frame.offset);
                        writer.commit();
                
                    }
                }
                
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct AfXdpClient{

}

impl AfXdpClient{
    pub fn new() -> Self{
        AfXdpClient{}
    }
    
}

fn ifinfo(ifname: &str, queue_id: Option<u32>) -> Result<IfInfo, xdpilone::Errno> {
    let mut bytes = String::from(ifname);
    bytes.push('\0');
    let bytes = bytes.as_bytes();
    let name = core::ffi::CStr::from_bytes_with_nul(bytes).unwrap();

    let mut info = IfInfo::invalid();
    info.from_name(name)?;
    if let Some(q) = queue_id {
        info.set_queue(q);
    }

    Ok(info)
}

fn prepare_buffer(offset: u64, buffer: &mut [u8]) -> XdpDesc {
    buffer[..ARP.len()].copy_from_slice(&ARP[..]);
    let length: u32 = 0;
    let extra = length.saturating_sub(ARP.len() as u32);

    XdpDesc {
        addr: offset,
        len: ARP.len() as u32 + extra,
        options: 0,
    }
}

#[rustfmt::skip]
static ARP: [u8; 14+28] = [
    0x11, 0x12, 0x13, 0x14, 0x15, 0x16,
    0x31, 0x32, 0x33, 0x34, 0x35, 0x36,
    0x08, 0x06,

    0x00, 0x01,
    0x08, 0x00, 0x06, 0x04,
    0x00, 0x01,
    0x11, 0x12, 0x13, 0x14, 0x15, 0x16,
    0x21, 0x22, 0x23, 0x24,
    0x31, 0x32, 0x33, 0x34, 0x35, 0x36,
    0x41, 0x42, 0x43, 0x44,
];