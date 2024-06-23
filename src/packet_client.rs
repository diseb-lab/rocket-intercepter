//! This module is responsible for making and handling requests to the controller.

use crate::packet_client::proto::{Config, GetConfig, PacketAck};
use log::{debug, info};
use proto::packet_service_client::PacketServiceClient;
use proto::{Packet, ValidatorNodeInfo};

pub mod proto {
    tonic::include_proto!("packet");
}

/// Struct that represents the object that is able to call the controller module.
#[derive(Debug)]
pub struct PacketClient {
    pub client: PacketServiceClient<tonic::transport::Channel>,
}

impl PacketClient {
    /// Initializes a new PacketClient that connects to the controller.
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let client = PacketServiceClient::connect("http://[::1]:50051").await?;
        Ok(Self { client })
    }

    /// Sends an intercepted message to the controller, asking for an action.
    ///
    /// # Parameters
    /// * 'packet_data' - the data of the intercepted message.
    /// * 'packet_from_port' - the port of the node where the message came from.
    /// * 'packet_to_port' - the port of the node where the message is sent to.
    pub async fn send_packet(
        &mut self,
        packet_data: Vec<u8>,
        packet_from_port: u32,
        packet_to_port: u32,
    ) -> Result<PacketAck, Box<dyn std::error::Error>> {
        if packet_data.is_empty() {
            return Err("Packet data is empty".into());
        }

        match packet_from_port {
            u32::MAX => return Err("packet_from_port not set properly".into()),
            port => port,
        };

        match packet_to_port {
            u32::MAX => return Err("packet_to_port not set properly".into()),
            port => port,
        };

        let packet = Packet {
            data: packet_data.clone(),
            from_port: packet_from_port,
            to_port: packet_to_port,
        };

        let request = tonic::Request::new(packet);

        let response = self.client.send_packet(request).await?.into_inner(); // we send to controller and are waiting for the response
        debug!(
            "action: {}, from_port: {}, to_port: {}, original_data: {}, possibly_mutated_data: {}",
            response.action,
            packet_from_port,
            packet_to_port,
            hex::encode(packet_data),
            hex::encode(&response.data),
        );

        Ok(response)
    }

    /// Sends the info of all ValidatorNodes to the controller.
    ///
    /// # Parameters
    /// * 'validator_node_info_list' - A list of all the info of the ValidatorNodes.
    pub async fn send_validator_node_info(
        &mut self,
        validator_node_info_list: Vec<ValidatorNodeInfo>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let request = tonic::Request::new(tokio_stream::iter(validator_node_info_list.into_iter()));
        let response = self
            .client
            .send_validator_node_info(request)
            .await?
            .into_inner();
        info!("Response: {:?}", response);

        Ok(response.status)
    }

    /// Sends a request to the controller asking for the network configuration.
    pub async fn get_config(&mut self) -> Result<Config, Box<dyn std::error::Error>> {
        let request = tonic::Request::new(GetConfig {});
        let response = self.client.get_config(request).await?.into_inner();
        info!("Response: {:?}", response);

        Ok(response)
    }
}

// Note: these tests require the controller to be running
#[cfg(test)]
mod integration_tests_grpc {
    use super::*;
    use crate::packet_client::proto::Partition;

    async fn setup() -> PacketClient {
        PacketClient::new().await.unwrap()
    }

    #[tokio::test]
    // #[coverage(off)]  // Only available in nightly build, don't forget to uncomment #![feature(coverage_attribute)] on line 1 of main
    async fn send_packet_ok() {
        let mut client = setup().await;
        let packet_data: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        // Call the async function and obtain the result
        let result = client.send_packet(packet_data, 60000, 60001).await;

        // Assert that the result is Ok
        assert!(
            result.is_ok(),
            "assertion failed, expected: result.is_ok(), but got: {:?}",
            result
        );
    }
    #[tokio::test]
    // #[coverage(off)]  // Only available in nightly build, don't forget to uncomment #![feature(coverage_attribute)] on line 1 of main
    async fn send_packet_empty_bytes() {
        let mut client = setup().await;
        // Prepare a request with invalid data
        let packet_data: Vec<u8> = vec![]; // Empty data

        // Call the async function and obtain the result
        let result = client.send_packet(packet_data, 2, 3).await;

        // Assert that the result is not Ok (i.e., Err)
        assert!(result.is_err());
    }

    #[tokio::test]
    // #[coverage(off)]  // Only available in nightly build, don't forget to uncomment #![feature(coverage_attribute)] on line 1 of main
    async fn send_packet_max_from_port() {
        let mut client = setup().await;
        // Prepare a request with invalid data
        let packet_data: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]; // Empty data
        let packet_from_port: u32 = u32::MAX;

        // Call the async function and obtain the result
        let result = client.send_packet(packet_data, packet_from_port, 3).await;

        // Assert that the result is not Ok (i.e., Err)
        assert!(result.is_err());
    }

    #[tokio::test]
    // #[coverage(off)]  // Only available in nightly build, don't forget to uncomment #![feature(coverage_attribute)] on line 1 of main
    async fn send_packet_max_to_port() {
        let mut client = setup().await;
        // Prepare a request with invalid data
        let packet_data: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]; // Empty data
        let packet_to_port: u32 = u32::MAX;

        // Call the async function and obtain the result
        let result = client.send_packet(packet_data, 2, packet_to_port).await;

        // Assert that the result is not Ok (i.e., Err)
        assert!(result.is_err());
    }

    #[tokio::test]
    // #[coverage(off)]  // Only available in nightly build, don't forget to uncomment #![feature(coverage_attribute)] on line 1 of main
    async fn validator_node_info_ok() {
        let mut client = setup().await;
        let validator_node_info_list = vec![ValidatorNodeInfo {
            peer_port: 60000,
            ws_public_port: 61000,
            ws_admin_port: 62000,
            rpc_port: 63000,
            status: "active".to_string(),
            validation_key: "READ SOIL DASH FUND ISLE LEN SOD OUT MACE ERIC DRAG MILT".to_string(),
            validation_private_key: "paAgnNZ9NaKTACGT3dGBV2eNHRxXNo8hRhNQNEWRJ23m5isp93t"
                .to_string(),
            validation_public_key: "n9KjTKEaHJ12Kuon5PDZ7fQAo5ExZ6cKH4h3L8q6m9YhoYqeBDho"
                .to_string(),
            validation_seed: "shM8uxbqE5g43G3VwKt6TM2pLvFan".to_string(),
        }];
        let result = client
            .send_validator_node_info(validator_node_info_list)
            .await;
        assert!(
            result.is_ok(),
            "assertion failed, expected: result.is_ok(), but got: {:?}",
            result
        );
        assert_eq!(result.unwrap(), "Received validator node info".to_string());
    }

    #[tokio::test]
    // #[coverage(off)]  // Only available in nightly build, don't forget to uncomment #![feature(coverage_attribute)] on line 1 of main
    async fn get_config_ok() {
        let partition = Partition {
            nodes: vec![0, 1, 2],
        };
        let config = Config {
            base_port_peer: 60000,
            base_port_ws: 61000,
            base_port_ws_admin: 62000,
            base_port_rpc: 63000,
            number_of_nodes: 3,
            partitions: vec![partition],
        };
        let mut client = setup().await;
        let result = client.get_config().await;
        assert!(
            result.is_ok(),
            "assertion failed, expected: result.is_ok(), but got: {:?}",
            result
        );
        assert_eq!(result.unwrap(), config);
    }
}
