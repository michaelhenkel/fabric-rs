use core::{mem::MaybeUninit, num::NonZeroU32, ptr::NonNull};
use std::{collections::HashMap, error::Error, sync::{atomic::{AtomicU32, Ordering}, Arc, Mutex}, time::Duration};
use anyhow::anyhow;
use pnet::packet::ethernet::EthernetPacket;
use tokio::sync::RwLock;
use xdpilone::{xdp::XdpDesc, DeviceQueue, RingRx, RingTx};
use aya::maps::{MapData, XskMap};
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
        info!("Sending packet to interface {}", ifidx);
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
    rx_map: HashMap<u32, Arc<RwLock<tokio::sync::mpsc::Receiver<Vec<u8>>>>>,
}

impl AfXdp{
    pub fn new(interface_list: HashMap<u32, Interface>, xsk_map: XskMap<MapData>) -> Self{
        let mut rx_map = HashMap::new();
        let mut tx_map = HashMap::new();
        for (ifidx, interface) in &interface_list{
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

        for (ifidx, interface) in interface_list{
            let recv_tx = recv_tx.clone();
            let xsk_map = self.xsk_map.clone();
            let rx = rx_map.remove(&ifidx).unwrap();
            
            let jh = tokio::spawn(async move{
                let _ = interface_runner(interface.clone(), xsk_map, rx.clone(), recv_tx).await;
                
            });
            jh_list.push(jh);

        }
        Ok(jh_list)
    }
}

pub async fn interface_runner(interface: Interface, xsk_map: Arc<Mutex<XskMap<MapData>>>, rx: Arc<RwLock<tokio::sync::mpsc::Receiver<Vec<u8>>>>, recv_tx: tokio::sync::mpsc::Sender<(u32,Vec<u8>)>) -> anyhow::Result<()>{
    let (device_queue, mut ring_rx, mut ring_tx, recv_desc, recv_buf, send_desc, send_buf) ={
        let alloc = Box::new(PacketMap(MaybeUninit::uninit()));
        let mem = NonNull::new(Box::leak(alloc).0.as_mut_ptr()).unwrap();
        let umem = unsafe { Umem::new(UmemConfig::default(), mem) }.unwrap();
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

        let ring_tx = rxtx.map_tx().unwrap();
        let ring_rx = rxtx.map_rx().unwrap();
        umem.bind(&rxtx).unwrap();
        let mut xsk_map = xsk_map.lock().unwrap();
        xsk_map.set(0, device_queue.as_raw_fd(), 0).unwrap();

        let mut recv_frame = umem.frame(BufIdx(0)).unwrap();
        {
            let mut writer = device_queue.fill(1);
            writer.insert_once(recv_frame.offset);
            writer.commit();
        }

        let recv_buf = unsafe { recv_frame.addr.as_mut() };

        let recv_desc = XdpDesc{
            addr: recv_frame.offset,
            len: 38,
            options: 0,
        };


        let mut send_frame = umem.frame(BufIdx(0)).unwrap();
 
        let send_buf = unsafe { send_frame.addr.as_mut() };

        let send_desc = XdpDesc{
            addr: send_frame.offset,
            len: 38,
            options: 0,
        };

        (device_queue, ring_rx, ring_tx, recv_desc, recv_buf, send_desc, send_buf)
    };

    let mut jh_list = Vec::new();

    let mut stall_count = 0;
    const WAKE_THRESHOLD: u32 = 1 << 4;
    let mut stall_threshold = WAKE_THRESHOLD;


    let jh = tokio::spawn(async move{
        let mut rx = rx.write().await;
        loop{
            while let Some(msg) = rx.recv().await{
                info!("Received message to send to socket. Length: {}", msg.len());
                let s = msg.as_slice();
                send_buf[..s.len()].copy_from_slice(s); 
                let mut writer = ring_tx.transmit(1);
                let res = writer.insert_once(send_desc);
                info!("insert ret: {}", res);
                writer.commit();
            }
        }
    });
    jh_list.push(jh);

    let mut interval = tokio::time::interval(Duration::from_millis(10));
    let device_queue_mutex = Arc::new(Mutex::new(device_queue));
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

    let mut interval = tokio::time::interval(Duration::from_secs(2));
    let jh = tokio::spawn(async move{
        loop{
            interval.tick().await;
            info!("Sending empty packet to client");
            //recv_tx.send((1,Vec::new())).await.unwrap();
        }
    });
    jh_list.push(jh);
    

    let jh = tokio::spawn(async move{
        loop{
            let mut receive = ring_rx.receive(1);
            let mut data = Vec::new();
            if let Some(desc) = receive.read(){
                data = recv_buf.to_vec();
                receive.release();
                {
                    let mut device_queue = device_queue_mutex.lock().unwrap();
                    let mut writer = device_queue.fill(1);
                    writer.insert_once(desc.addr);
                    writer.commit();
                }
            } 
            if data.len() == 0{
                continue;
            }
            let (eth, buf) = data.as_slice().split_at(14);
            match EthernetPacket::new(eth){
                Some(eth_packet) => {
                    info!("Received packet: {:?}", eth_packet);
                    match recv_tx.try_send((interface.ifidx, data)){
                        Ok(_) => {
                            info!("Sent packet to client");
                        },
                        Err(e) => {
                            info!("Failed to send packet to client: {}", e);
                        }
                    
                    };
                    info!("Sent packet to client");
                },
                None => {
                    info!("Failed to parse ethernet packet");
                }
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

/*
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

*/