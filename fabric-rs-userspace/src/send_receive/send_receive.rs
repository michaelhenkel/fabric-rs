use std::{collections::HashMap, sync::Arc};
use log::{error, info, warn};
use tokio::sync::RwLock;
use crate::interface::interface::Interface;

#[derive(Clone)]
pub struct SendReceiveClient{
    sender_channel: tokio::sync::mpsc::Sender<(u32, Vec<u8>)>,
}

impl SendReceiveClient{
    pub async fn send(&self, ifidx: u32, msg: Vec<u8>) -> anyhow::Result<()>{
        info!("sending message on interface: {}", ifidx);
        self.sender_channel.send((ifidx,msg)).await?;
        Ok(())
    }
}

pub struct SendReceive{
    interfaces: HashMap<u32, Interface>,
    client: SendReceiveClient,
    tx: tokio::sync::mpsc::Sender<(u32, Vec<u8>)>,
    recv_rx: Arc<RwLock<tokio::sync::mpsc::Receiver<(u32, Vec<u8>)>>>
}

impl SendReceive{

    pub fn client(&self) -> SendReceiveClient{
        self.client.clone()
    }

    pub fn new(interfaces: HashMap<u32, Interface>, tx: tokio::sync::mpsc::Sender<(u32, Vec<u8>)>) -> SendReceive{
        let (recv_tx, recv_rx) = tokio::sync::mpsc::channel(100);
        SendReceive{
            interfaces,
            client: SendReceiveClient{
                sender_channel: recv_tx,
            },
            tx,
            recv_rx: Arc::new(RwLock::new(recv_rx)),
        }
    }

    pub async fn run(&mut self) -> anyhow::Result<Vec<tokio::task::JoinHandle<()>>>{
        info!("Starting SendReceive");
        let (send_tx, send_rx) = tokio::sync::mpsc::channel(100);
        let mut jh_list = Vec::new();
        let jh = self.send_recv(send_tx, self.recv_rx.clone());
        jh_list.extend(jh.await?);
        let jh = self.recv(send_rx);
        jh_list.push(jh.await?);
        Ok(jh_list)
    }

    pub async fn recv(&mut self, mut rx: tokio::sync::mpsc::Receiver<(u32, Vec<u8>)>) -> anyhow::Result<tokio::task::JoinHandle<()>>{
        let tx = self.tx.clone();
        let jh = tokio::spawn(async move{
            loop{
                while let Some(msg) = rx.recv().await{
                    if let Err(e) = tx.send(msg.clone()).await{
                        error!("Error sending value: {:?}", e);
                    }
                    info!("message sent");
                }
            }
        });
        Ok(jh)
    }
    async fn send_recv(&mut self, tx: tokio::sync::mpsc::Sender<(u32, Vec<u8>)>, rx: Arc<RwLock<tokio::sync::mpsc::Receiver<(u32, Vec<u8>)>>>) -> anyhow::Result<Vec<tokio::task::JoinHandle<()>>>{
        let mut jh_list = Vec::new();
        let all_interfaces = pnet::datalink::interfaces();
        let mut sender_list = HashMap::new();
        for (ifidx, interface) in &self.interfaces{
            let data_link_interface = if let Some(interface) = all_interfaces
                .iter()
                .find(|e| e.index == interface.ifidx){
                    interface
            } else {
                panic!("interface not found");
            };

            let(dl_tx, mut dl_rx) = match pnet::datalink::channel(data_link_interface, Default::default()) {
                Ok(pnet::datalink::Channel::Ethernet(tx, rx)) => (tx, rx),
                Ok(_) => panic!("Unknown channel type"),
                Err(e) => panic!("Error happened {}", e),
            };
            sender_list.insert(interface.ifidx, dl_tx);
            let tx = tx.clone();
            let ifidx = interface.ifidx.clone();
            let jh = tokio::spawn(async move{
                loop{
                    let buf = dl_rx.next().unwrap();
                    info!("received message on interface: {}", ifidx);
                    tx.send((ifidx, buf.to_vec())).await.unwrap();
                }
            });
            jh_list.push(jh);
        }

        let jh = tokio::spawn(async move{
            let mut rx = rx.write().await;
            loop{
                while let Some((ifidx, msg)) = rx.recv().await{
                    if let Some(tx) = sender_list.get_mut(&ifidx){
                        if let Some(res) = tx.send_to(msg.as_slice(), None) {
                            match res {
                                Ok(_) => {
                                    info!("message sent");
                                },
                                Err(e) => {
                                    panic!("failed to send packet: {:?}", e);
                                }
                            }
                        } else {
                            panic!("failed to send packet");
                        }
                    }
                }
            }
        });
        jh_list.push(jh);
        Ok(jh_list)
    }
}