use core::{mem::MaybeUninit, num::NonZeroU32, ptr::NonNull};
use std::{collections::HashMap, error::Error, sync::{atomic::{AtomicU32, Ordering}, Arc, Mutex}, time::Duration};
use anyhow::anyhow;
use disco_rs::DiscoHdrPacket;
use fabric_rs_common::InterfaceQueue;
use pnet::packet::{ethernet::EthernetPacket, Packet};
use tokio::sync::RwLock;
use xdpilone::{xdp::XdpDesc, DeviceQueue, RingRx, RingTx};
use aya::maps::{MapData, XskMap, HashMap as BpfHashMap};
use xdpilone::{BufIdx, IfInfo, Socket, SocketConfig, Umem, UmemConfig};
use log::info;
use crate::interface::interface::Interface;



#[repr(align(4096))]
struct PacketMap(MaybeUninit<[u8; 1 << 20]>);

#[derive(Clone)]
pub struct AfXdpClient{
    tx_map: HashMap<u32, tokio::sync::mpsc::Sender<Vec<u8>>>,
}

impl AfXdpClient{
    pub fn new(tx_map: HashMap<u32, tokio::sync::mpsc::Sender<Vec<u8>>>) -> Self{
        AfXdpClient{
            tx_map,
        }
    }
    pub async fn send(&mut self, ifidx: u32, buf: Vec<u8>) -> anyhow::Result<()>{
        if let Some(tx) = self.tx_map.get_mut(&ifidx){
            tx.send(buf).await.unwrap();
        }
        Ok(())
    }
}

pub struct AfXdp{
    client: AfXdpClient,
    interface_list: HashMap<u32, Interface>,
    xsk_map: Arc<Mutex<XskMap<MapData>>>,
    interface_queue_table: Arc<Mutex<BpfHashMap<MapData, InterfaceQueue, u32>>>,
    rx_map: HashMap<u32, Arc<RwLock<tokio::sync::mpsc::Receiver<Vec<u8>>>>>,
}

impl AfXdp{
    pub fn new(interface_list: HashMap<u32, Interface>, xsk_map: XskMap<MapData>, interface_queue_table: BpfHashMap<MapData, InterfaceQueue, u32>) -> Self{
        let mut rx_map = HashMap::new();
        let mut tx_map = HashMap::new();
        for (ifidx, _interface) in &interface_list{
            let (tx, rx) = tokio::sync::mpsc::channel(100);
            rx_map.insert(*ifidx, Arc::new(RwLock::new(rx)));
            tx_map.insert(*ifidx,tx);
        }
        AfXdp{
            client: AfXdpClient::new(
                tx_map
            ),
            interface_list,
            xsk_map: Arc::new(Mutex::new(xsk_map)),
            interface_queue_table: Arc::new(Mutex::new(interface_queue_table)),
            rx_map
        }
    }
    pub fn client(&self) -> AfXdpClient{
        self.client.clone()
    }
    pub async fn run(&mut self, recv_tx: tokio::sync::mpsc::Sender<(u32, Vec<u8>)>) -> anyhow::Result<Vec<tokio::task::JoinHandle<()>>>{
        let mut rx_map = self.rx_map.clone();
        let interface_list = self.interface_list.clone();

        let mut jh_list = Vec::new();

        let mut idx = 0;

        for (ifidx, interface) in interface_list{
            let recv_tx = recv_tx.clone();
            let xsk_map = self.xsk_map.clone();
            let interface_queue_table = self.interface_queue_table.clone();
            let rx = rx_map.remove(&ifidx).unwrap();
            
            let jh = tokio::spawn(async move{
                let _ = interface_runner(interface.clone(), xsk_map, interface_queue_table, rx.clone(), recv_tx, idx).await;
                
            });
            jh_list.push(jh);
            idx += 1;

        }
        Ok(jh_list)
    }
}

pub async fn interface_runner(
    interface: Interface,
    xsk_map: Arc<Mutex<XskMap<MapData>>>,
    interface_queue_table: Arc<Mutex<BpfHashMap<MapData, InterfaceQueue, u32>>>,
    rx: Arc<RwLock<tokio::sync::mpsc::Receiver<Vec<u8>>>>,
    recv_tx: tokio::sync::mpsc::Sender<(u32,Vec<u8>)>,
    idx: u32,
) -> anyhow::Result<()>{

    let frame_number = 10;

    let (fq_cq, mut ring_rx, mut ring_tx, recv_buf, mut send_desc, send_buf) ={
        let alloc = Box::new(PacketMap(MaybeUninit::uninit()));
        let mem = NonNull::new(Box::leak(alloc).0.as_mut_ptr()).unwrap();
        let umem = unsafe { Umem::new(UmemConfig::default(), mem) }.unwrap();
        let info = ifinfo(&interface.name, Some(0)).unwrap();
        let sock = Socket::with_shared(&info, &umem).unwrap();
        let mut fq_cq = umem.fq_cq(&sock).unwrap();
        let rxtx = umem
        .rx_tx(
            &sock,
            &SocketConfig {
                rx_size: NonZeroU32::new(1 << 11),
                tx_size: NonZeroU32::new(1 << 14),
                bind_flags: SocketConfig::XDP_BIND_NEED_WAKEUP,
                //bind_flags: SocketConfig::XDP_BIND_ZEROCOPY | SocketConfig::XDP_BIND_NEED_WAKEUP,
            },
        ).unwrap();

        let ring_tx = rxtx.map_tx().unwrap();
        let ring_rx = rxtx.map_rx().unwrap();
        umem.bind(&rxtx).unwrap();

        let interface_queue = InterfaceQueue::new(interface.ifidx, 0);
        let mut interface_queue_table = interface_queue_table.lock().unwrap();
        interface_queue_table.insert(interface_queue, idx, 0).unwrap();

        let mut xsk_map = xsk_map.lock().unwrap();
        xsk_map.set(idx, fq_cq.as_raw_fd(), 0).unwrap();

        let mut frame = umem.frame(BufIdx(0)).unwrap();

        {
            let mut writer = fq_cq.fill(1);
            writer.insert_once(frame.offset);
            writer.commit();
        }

        let recv_buf = unsafe { frame.addr.as_mut() };
        let mut send_frame = umem.frame(BufIdx(0)).unwrap();
 
        let send_buf = unsafe { send_frame.addr.as_mut() };

        let send_desc = XdpDesc{
            addr: send_frame.offset,
            len: 0,
            options: 0,
        };
        (fq_cq, ring_rx, ring_tx, recv_buf, send_desc, send_buf)
    };

    let mut jh_list = Vec::new();

    let mut stall_count = 0;
    const WAKE_THRESHOLD: u32 = 1 << 4;
    let mut stall_threshold = WAKE_THRESHOLD;


    let jh = tokio::spawn(async move{
        let mut rx = rx.write().await;
        loop{
            while let Some(msg) = rx.recv().await{
                let s = msg.as_slice();
                info!("SEND PACKET SIZE: {}", s.len());
                send_buf[..s.len()].copy_from_slice(s); 
                let mut writer = ring_tx.transmit(1);
                send_desc.len = s.len() as u32;
                writer.insert_once(send_desc);
                writer.commit();
            }
        }
    });
    jh_list.push(jh);

    let mut interval = tokio::time::interval(Duration::from_millis(10));
    let device_queue_mutex = Arc::new(Mutex::new(fq_cq));
    let device_queue = device_queue_mutex.clone();
    let jh = tokio::spawn(async move{
        loop{
            tokio::select! {
                _ = interval.tick() => {
                    let comp_now: u32;
                    let mut device_queue = device_queue.lock().unwrap();
                    {
                        let mut reader = device_queue.complete(1);
                        let mut comp_temp = 0;
                        while reader.read().is_some() {
                            comp_temp += 1;
                        }
                        comp_now = comp_temp;
                        reader.release();
                    }
                    if comp_now == 0 {
                        stall_count += 1;
                    }
                    if stall_count > stall_threshold {
                        device_queue.wake();
                        stall_threshold += WAKE_THRESHOLD;
                    }
                },
            }
        }
    });
    jh_list.push(jh);

    let jh = tokio::task::spawn_blocking(move ||{
        loop{
            let mut receive = ring_rx.receive(frame_number);
            let mut frame_idx = 0;
            while let Some(desc) = receive.read(){
                let buf = &recv_buf.as_ref()[desc.addr as usize..(desc.addr as usize + desc.len as usize)];
                let eth_packet = if let Some(eth_packet) = EthernetPacket::new(&buf){
                    eth_packet
                } else {
                    info!("Failed to parse ethernet packet");
                    continue;
                };
                let disco_hdr = if let Some(disco_hdr) = DiscoHdrPacket::new(&eth_packet.payload()){
                    disco_hdr
                } else {
                    info!("Failed to parse disco packet");
                    continue;
                };
                if disco_hdr.get_op() == 1 {
                    info!("XDP Received packet size: {}", buf.len());
                    info!("XDP Received disco packet: {}", disco_hdr);
                }

                let data = buf.to_vec();
                

                receive.release();
                {
                    let mut device_queue = device_queue_mutex.lock().unwrap();
                    let mut writer = device_queue.fill(frame_number);
                    writer.insert_once(desc.addr);
                    writer.commit();
                }
                if let Err(e) = recv_tx.blocking_send((interface.ifidx, data)){
                    info!("Failed to send packet to client: {}", e);
                }
                frame_idx += 1;
            } 
        }
    });
    jh_list.push(jh);
    
    futures::future::join_all(jh_list).await;

    Ok(())
}

fn ifinfo(ifname: &str, queue_id: Option<u32>) -> Result<IfInfo, anyhow::Error> {
    let mut bytes = String::from(ifname);
    bytes.push('\0');
    let bytes = bytes.as_bytes();
    let name = core::ffi::CStr::from_bytes_with_nul(bytes).unwrap();

    let mut info = IfInfo::invalid();
    if let Err(e) = info.from_name(name){
        return Err(anyhow!("Failed to get interface info: {}", e));
    }
    if let Some(q) = queue_id {
        info.set_queue(q);
    }

    Ok(info)
}

enum DeviceCommand{
    FILL(XdpDesc),
    COMPLETE,
}