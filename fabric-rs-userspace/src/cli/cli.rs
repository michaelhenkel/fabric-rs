use std::{process::exit, sync::Arc};
use inquire::{error::CustomUserError, required, Text};
use log::{error, info};
use tokio::{sync::RwLock, task::JoinHandle};

use crate::state::{state::{KeyValue, StateClient}, table::table::TableType};

pub struct Cli{
    client: CliClient,
    rx: Arc<RwLock<tokio::sync::mpsc::Receiver<CliCommand>>>,
}

pub enum CliCommand{
    List{
        table_type: TableType,
        tx: tokio::sync::oneshot::Sender<Vec<KeyValue>>
    },
}

impl Cli{
    pub fn new() -> Cli{
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        Cli{
            client: CliClient::new(tx),
            rx: Arc::new(RwLock::new(rx)),
        }
    }
    pub async fn run(&self, state_client: StateClient) -> anyhow::Result<Vec<JoinHandle<()>>>{
        info!("Starting Cli");
        let mut jh_list = Vec::new();
        let rx = self.rx.clone();
        let jh = tokio::spawn(async move{
            let mut rx = rx.write().await;
            loop{
                while let Some(cmd) = rx.recv().await{
                    match cmd{
                        CliCommand::List{table_type, tx} => {
                            info!("cli command list table_type: {}", table_type);
                            let nt = match state_client.list(table_type).await{
                                Ok(nt) => nt,
                                Err(e) => {
                                    error!("Error listing table: {}", e);
                                    continue;
                                },
                            };
                            if let Err(_e) = tx.send(nt){
                                error!("Error sending table");
                            };
                        },

                    }
                }
            }
        });
        jh_list.push(jh);

        let cli_client = self.client.clone();

        let jh = tokio::spawn(async move {
            loop{
                let command = Text::new("Cmd -> ")
                .with_autocomplete(&suggester)
                .with_validator(required!())
                //.with_validator(length!(10))
                .prompt()
                .unwrap();
                match command.as_str() {
                    "RouteTable" => {
                        cli_client.list(TableType::RouteTable).await.unwrap().iter().for_each(|kv| println!("{}", kv));
                    },
                    "NeighborTable" => {
                        cli_client.list(TableType::NeighborTable).await.unwrap().iter().for_each(|kv| println!("{}", kv));
                    },
                    "ForwardingTable" => {
                        cli_client.list(TableType::ForwardingTable).await.unwrap().iter().for_each(|kv| println!("{}", kv));
                    },
                    "Exit" => {
                        exit(0)
                    },
                    _ => {
                        println!("Command not found");
                    }
                }
                
            }
        });
        jh_list.push(jh);
        Ok(jh_list)
    }

    pub fn client(&self) -> CliClient{
        self.client.clone()
    }
}

fn suggester(val: &str) -> Result<Vec<String>, CustomUserError> {
    let suggestions = [
        "RouteTable",
        "NeighborTable",
        "ForwardingTable",
        "Exit"
    ];

    let val_lower = val.to_lowercase();

    Ok(suggestions
        .iter()
        .filter(|s| s.to_lowercase().contains(&val_lower))
        .map(|s| String::from(*s))
        .collect())
}

#[derive(Clone)]
pub struct CliClient{
    tx: tokio::sync::mpsc::Sender<CliCommand>,
}

impl CliClient{
    pub fn new(tx: tokio::sync::mpsc::Sender<CliCommand>) -> Self{
        CliClient{tx}
    }
    pub async fn list(&self, table_type: TableType) -> anyhow::Result<Vec<KeyValue>>{
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx.send(CliCommand::List{table_type, tx}).await?;
        let nt = rx.await?;
        Ok(nt)
    }
}