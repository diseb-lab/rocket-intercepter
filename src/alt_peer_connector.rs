use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use bytes::{Buf, BytesMut};
use log::{debug, error};
use openssl::ssl::{Ssl, SslContext, SslMethod};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_openssl::SslStream;

/// The same as peer_connector.rs, but this makes it possible to
/// send the peer numbers to the controller if we need that at one point

pub struct PeerConnector<'a> {
    pub ip_addr: &'a str,
    pub base_port: u16
}

impl<'a> PeerConnector<'a> {
    /// Connect 2 peers with predefined ip-address and base port
    pub async fn connect_peers(&self, peer1: u16, peer2: u16, pub_key1: &str, pub_key2: &str)
                               -> (JoinHandle<()>, JoinHandle<()>) {
        (PeerConnection { peer1, peer2 }).connect(self.ip_addr, self.base_port, pub_key1, pub_key2).await
    }
}

pub struct PeerConnection {
    pub peer1: u16,
    pub peer2: u16
}

impl PeerConnection {
    /// Connect 2 peers together
    /// Established SSL streams between the peers
    /// Returns the handling of the messages sent over these streams as 2 threads
    pub async fn connect(&self, ip_addr: &str, base_port: u16, pub_key1: &str, pub_key2: &str)
                         -> (JoinHandle<()>, JoinHandle<()>) {
        let ssl_stream_1 = self.create_ssl_stream(&ip_addr, base_port+&self.peer1, &pub_key2).await;
        let ssl_stream_2 = self.create_ssl_stream(&ip_addr, base_port+&self.peer2, &pub_key1).await;
        self.handle_peer_connections(ssl_stream_1, ssl_stream_2).await
    }

    /// Create an SSL stream from a peer to another peer
    /// Uses the current peer's ip+port and the other peer's public key
    async fn create_ssl_stream(&self, ip: &str, port: u16, pub_key_peer: &str) -> SslStream<TcpStream> {
        let socket_address = SocketAddr::new(IpAddr::from_str(ip).unwrap(), port);
        let tcp_stream = match TcpStream::connect(socket_address).await {
            Ok(tcp_stream) => tcp_stream,
            Err(e) => panic!("{}", e),
        };

        tcp_stream.set_nodelay(true).expect("Set nodelay failed");
        let ssl_ctx = SslContext::builder(SslMethod::tls()).unwrap().build();
        let ssl = Ssl::new(&ssl_ctx).unwrap();
        let mut ssl_stream = SslStream::<TcpStream>::new(ssl, tcp_stream).unwrap();
        SslStream::connect(Pin::new(&mut ssl_stream))
            .await
            .expect("SSL connection failed");

        let content = Self::format_upgrade_request_content(&pub_key_peer);
        ssl_stream
            .write_all(content.as_bytes())
            .await
            .expect("Could not send XRPL handshake request");

        let mut buf = BytesMut::new();
        let mut vec = vec![0; 4096];
        let size = ssl_stream
            .read(&mut vec)
            .await
            .expect("Unable to read handshake response");
        vec.resize(size, 0);
        buf.extend_from_slice(&vec);

        if size == 0 {
            error!("Current buffer: {}", String::from_utf8_lossy(&buf).trim());
            panic!("Socket closed");
        }

        if let Some(n) = buf.windows(4).position(|x| x == b"\r\n\r\n") {
            let mut headers = [httparse::EMPTY_HEADER; 32];
            let mut resp = httparse::Response::new(&mut headers);
            let status = resp.parse(&buf[0..n + 4]).expect("Response parse failed");
            if status.is_partial() { panic!("Invalid headers"); }

            let response_code = resp.code.unwrap();
            debug!("Peer Handshake Response: version {}, status {}, reason {}",
                resp.version.unwrap(),
                resp.code.unwrap(),
                resp.reason.unwrap());
            debug!("Printing response headers:");
            for header in headers.iter().filter(|h| **h != httparse::EMPTY_HEADER) {
                debug!("{}: {}", header.name, String::from_utf8_lossy(header.value));
            }

            buf.advance(n + 4);

            if response_code != 101 && ssl_stream.read_to_end(&mut buf.to_vec()).await.unwrap() == 0 {
                debug!("Body: {}", String::from_utf8_lossy(&buf).trim());
            }

            if !buf.is_empty() {
                debug!("Current buffer is not empty?: {}", String::from_utf8_lossy(&buf).trim());
                panic!("Buffer should be empty, are the peer slots full?");
            }
        }

        ssl_stream
    }

    fn format_upgrade_request_content(pub_key_peer: &str) -> String {
        format!(
            "\
            GET / HTTP/1.1\r\n\
            Upgrade: XRPL/2.2\r\n\
            Connection: Upgrade\r\n\
            Connect-As: Peer\r\n\
            Public-Key: {}\r\n\
            Session-Signature: a\r\n\
            \r\n",
            pub_key_peer
        )
    }

    /// Handle the connection between 2 peers
    /// Returns 2 threads which continuously handle incoming messages
    async fn handle_peer_connections(&self, ssl_stream_1: SslStream<TcpStream>, ssl_stream_2: SslStream<TcpStream>)
                                     -> (JoinHandle<()>, JoinHandle<()>){
        let arc_stream1_0 = Arc::new(Mutex::new(ssl_stream_1));
        let arc_stream2_0 = Arc::new(Mutex::new(ssl_stream_2));

        let arc_stream1_1 = arc_stream1_0.clone();
        let arc_stream2_1 = arc_stream2_0.clone();

        let thread_1 = tokio::spawn(async move {
            loop {
                &self.handle_message(&arc_stream1_0, &arc_stream2_0).await;
                debug!("Forwarded peer message 1->2")
            }
        });

        let thread_2 = tokio::spawn(async move {
            loop {
                &self.handle_message(&arc_stream2_1, &arc_stream1_1).await;
                debug!("Forwarded peer message 2->1")
            }
        });

        (thread_1, thread_2)
    }

    /// Handles incoming messages from the 'form' stream to the 'to' stream.
    /// Utilizes the controller module to determine new packet contents and action
    async fn handle_message(&self, from: &Arc<Mutex<SslStream<TcpStream>>>, to: &Arc<Mutex<SslStream<TcpStream>>>) {
        let mut buf = BytesMut::with_capacity(64 * 1024);
        buf.resize(64 * 1024, 0);
        let size = from
            .lock()
            .await
            .read(buf.as_mut())
            .await
            .expect("Could not read from SSL stream");
        buf.resize(size, 0);
        if size == 0 {
            error!("Current buffer: {}", String::from_utf8_lossy(&buf).trim());
            return;
        }
        let bytes = buf.to_vec();
        if bytes[0] & 0x80 != 0 {
            error!("{:?}", bytes[0]);
            panic!("Received compressed message");
        }

        if bytes[0] & 0xFC != 0 { error!("Unknown version header"); }

        // TODO: send the message to the controller
        // TODO: use returned information for further execution
        // Send the 2 peers (the numbers) with the request
        // Controller should have information on the network itself



        // For now: send the raw bytes without processing to the receiver
        to.lock()
            .await
            .write_all(&buf)
            .await
            .expect("Could not write to SSL stream");
    }
}


