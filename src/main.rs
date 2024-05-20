mod docker_manager;
mod packet_client;
mod peer_connector;
use crate::peer_connector::PeerConnector;
use std::env;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> io::Result<()> {
    env::set_var("RUST_LOG", "DEBUG");
    env_logger::init();

    // Init docker network
    let network_config = docker_manager::get_config();
    let mut network = docker_manager::DockerNetwork::new(network_config);
    network.initialize_network().await;
    let client = match packet_client::PacketClient::new().await {
        Ok(client) => Arc::new(Mutex::new(client)),
        Err(e) => {
            // Handle the error here. For example, you can log the error and exit the program.
            eprintln!("Failed to create PacketClient: {}", e);
            std::process::exit(1);
        }
    };

    tokio::time::sleep(Duration::from_secs(3)).await;

    let peer_connector = PeerConnector::new("127.0.0.1".to_string());

    // Iterate over every unique validator node pair and create a thread for each
    let mut threads = Vec::new();
    for (i, container1) in network.containers.iter().enumerate() {
        for container2 in &network.containers[(i + 1)..network.containers.len()] {
            let (t1, t2) = peer_connector
                .clone()
                .connect_peers(
                    client.clone(),
                    container1.port_peer,
                    container2.port_peer,
                    container1.key_data.validation_public_key.as_str(),
                    container2.key_data.validation_public_key.as_str(),
                )
                .await;

            threads.push(t1);
            threads.push(t2);
        }
    }

    // Wait for all threads to exit (due to error)
    for t in threads {
        t.await.expect("Thread failed");
    }

    network.stop_network().await;

    Ok(())
}
