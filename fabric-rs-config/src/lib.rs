use futures::TryStreamExt;
use k8s_openapi::serde_json::error;
use kube_virt_rs::{
    flowtable::flowtable::{Flowtable, FlowtableStatus, Flow},
    resource::resource::ResourceClient,
    interface::interface::{Interface, InterfaceStatus},
};
use kube::{
    core::{crd::CustomResourceExt, ObjectMeta},
    Client,
    Api,
    runtime::{watcher, WatchStreamExt},
    Resource, api::ListParams,
};
use clap::{Parser, ValueEnum};
use log::{info, warn, error};

pub struct ConfigClient{
    instance: String,
    instance_type: InstanceType,
}
#[derive(ValueEnum, Debug, PartialEq, Clone, serde::Deserialize)]
pub enum InstanceType{
    INSTANCE,
    NETWORK,
}

impl ConfigClient{
    pub fn new(instance: String, instance_type: InstanceType) -> Self{
        Self{
            instance,
            instance_type,
        }
    }
    pub async fn get_flows(&self) -> anyhow::Result<Vec<Flow>> {
        let client = Client::try_default().await?;
        let api = Api::<Flowtable>::default_namespaced(client);
        let flows = match api.get(&self.instance).await{
            Ok(flow_table) => {
                let ftl = if let Some(status) = &flow_table.status{
                    status.flows.iter().map(|flow| {
                        flow.clone()
                    }).collect()
                } else {
                    Vec::new()
                };
                ftl
            },
            Err(e) => {
                error!("failed to get flow table: {:?}", e);
                return Err(anyhow::anyhow!("failed to get flow table: {:?}", e));
            }
        };
        Ok(flows)
    }
    pub async fn get_interfaces(&self) -> anyhow::Result<Vec<String>>{
        let client = Client::try_default().await?;
        let api = Api::<Interface>::default_namespaced(client);
        let lp = match self.instance_type{
            InstanceType::INSTANCE => {
                ListParams::default().labels(&format!("virt.dev/instance={}", self.instance))
            }
            InstanceType::NETWORK => {
                ListParams::default().labels(&format!("virt.dev/network={}", self.instance))
            }
        };
        let interface_list = match api.list(&lp).await{
            Ok(interfaces) => {
                let il: Vec<String> = interfaces.items.iter().map(|interface| {
                    match self.instance_type{
                        InstanceType::INSTANCE => {
                            interface.spec.name.clone()
                        },
                        InstanceType::NETWORK => {
                            interface.status.as_ref().unwrap().state.host_name.clone()
                        },
                    }
                    
                }).collect();
                il
            },
            Err(e) => {
                error!("failed to get interfaces: {:?}", e);
                return Err(anyhow::anyhow!("failed to get interfaces: {:?}", e));
            }
        };
        Ok(interface_list)
    }
    pub async fn run(&self) -> anyhow::Result<()>{
        let client = Client::try_default().await?;
        let api = Api::<Flowtable>::default_namespaced(client);
        let watcher_config = watcher::Config::default();
        watcher(api, watcher_config)
        .applied_objects()
        .default_backoff()
        .try_for_each(|flow_table| async move {
            if let Some(flow_table_name) = &flow_table.meta().name  {
                info!("flow table: {:?}", flow_table_name);
                if flow_table_name == &self.instance{
                    if let Some(status) = flow_table.status{
                        //if let Err(e) = self.tx.send(status).await{
                        //    error!("failed to send flow table: {:?}", e);
                        //}
                    }
                }
            }
            Ok(())
        })
        .await?;

        Ok(())
    }
}
