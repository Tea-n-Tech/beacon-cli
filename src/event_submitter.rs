#[path = "./data_collection.rs"]
mod data_collection;

use data_collection::proto::event_service_client::EventServiceClient;
use tokio::sync::mpsc;
use tonic::transport::Channel;

use crate::ClientCli;

pub struct EventSubmitter {
    client: EventServiceClient<Channel>,
    submission_handler: Option<tokio::task::JoinHandle<()>>,
    cli: ClientCli,
}

impl Drop for EventSubmitter {
    fn drop(&mut self) {
        match self.submission_handler {
            Some(ref mut submission_handler) => {
                submission_handler.abort();
            }
            None => {}
        }
    }
}

impl EventSubmitter {
    pub async fn new(cli: ClientCli) -> Result<Self, tonic::transport::Error> {
        match EventServiceClient::connect(format!(
            "http://{}:{}",
            cli.server_address, cli.server_port
        ))
        .await
        {
            Ok(client) => Ok(Self {
                client,
                submission_handler: None,
                cli,
            }),
            Err(e) => {
                eprintln!("Error connecting to event service: {:?}", e);
                Err(e)
            }
        }
    }

    pub async fn submit_events(&mut self) -> Result<(), ()> {
        let (tx, mut rx) = mpsc::channel::<data_collection::proto::ChangeEventBatch>(32);

        println!("Fetching initial state");
        let initial_state_result = self.client.initial_state(tonic::Request::new(())).await;
        if let Err(err) = &initial_state_result {
            eprintln!("Failed to get initial state: {}", err);
            return Err(());
        }

        let initial_state = initial_state_result.unwrap().into_inner();
        println!("Got initial state: {:?}", initial_state);

        // collect data indefinitely and send data to the channel
        self.submission_handler = Some(tokio::task::spawn(async move {
            data_collection::collect_events(tx, initial_state).await;
        }));

        loop {
            match rx.recv().await {
                Some(event_batch) => {
                    println!("Sending events {:?}", event_batch);
                    let request = tonic::Request::new(event_batch);
                    match self.client.send_events(request).await {
                        Ok(response) => {
                            println!("RESPONSE={:?}", response);
                        }
                        Err(e) => {
                            eprintln!("Error sending events: {:?}", e);
                            return Err(());
                        }
                    }
                }
                None => {}
            }
        }
    }
}
