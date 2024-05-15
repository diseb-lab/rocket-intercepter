use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use bytes::{Buf, BytesMut};
use log::{debug, error};
use openssl::ssl::{Ssl, SslContext, SslMethod};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_openssl::SslStream;

/// The lifetime specifier 'a is needed to make sure that
/// the reference to ip_addr stays alive while this object is alive
pub struct PeerConnector<'a> {
    pub ip_addr: &'a str,
    pub base_port: u16
}

impl<'a> PeerConnector<'a> {
    /// Connect 2 peers
    /// Established SSL streams between the peers
    /// Returns the handling of the messages sent over these streams as 2 threads
    pub async fn connect_peers(&self, peer1: u16, peer2: u16, pub_key1: &str, pub_key2: &str)
        -> (JoinHandle<()>, JoinHandle<()>) {
        let ssl_stream_1 = Self::create_ssl_stream(self.ip_addr, self.base_port+&peer1, &pub_key2).await;
        let ssl_stream_2 = Self::create_ssl_stream(self.ip_addr, self.base_port+&peer2, &pub_key1).await;
        Self::handle_peer_connections(ssl_stream_1, ssl_stream_2, peer1, peer2).await
    }

    /// Create an SSL stream from a peer to another peer
    /// Uses the current peer's ip+port and the other peer's public key
    async fn create_ssl_stream(ip: &str, port: u16, pub_key_peer: &str) -> SslStream<TcpStream> {
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
    async fn handle_peer_connections(ssl_stream_1: SslStream<TcpStream>, ssl_stream_2: SslStream<TcpStream>,
                                    peer1: u16, peer2: u16)
        -> (JoinHandle<()>, JoinHandle<()>){
        let arc_stream1_0 = Arc::new(Mutex::new(ssl_stream_1));
        let arc_stream2_0 = Arc::new(Mutex::new(ssl_stream_2));

        let arc_stream1_1 = arc_stream1_0.clone();
        let arc_stream2_1 = arc_stream2_0.clone();

        let thread_1 = tokio::spawn(async move {
            loop {
                Self::handle_message(&arc_stream1_0, &arc_stream2_0, peer1, peer2).await;
            }
        });

        let thread_2 = tokio::spawn(async move {
            loop {
                Self::handle_message(&arc_stream2_1, &arc_stream1_1, peer2, peer1).await;
            }
        });

        (thread_1, thread_2)
    }

    /// Handles incoming messages from the 'form' stream to the 'to' stream.
    /// Utilizes the controller module to determine new packet contents and action
    async fn handle_message(from: &Arc<Mutex<SslStream<TcpStream>>>, to: &Arc<Mutex<SslStream<TcpStream>>>,
                            peer_from: u16, peer_to:u16) {
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

        let start_time = Instant::now();

        // Delay functionality
        // For now peer1 gets delayed for 500ms
        if peer_from == 1 {
            Self::delay_execution(start_time, 500).await;
        }

        // For now: send the raw bytes without processing to the receiver
        to.lock()
            .await
            .write_all(&buf)
            .await
            .expect("Could not write to SSL stream");

        debug!("Forwarded peer message {} -> {}", peer_from, peer_to)
    }

    async fn delay_execution(start_time: Instant, ms: u64) {
        let elapsed_time = start_time.elapsed();
        let delay_duration = Duration::from_millis(ms) - elapsed_time;

        debug!("Delay peer");

        if delay_duration > Duration::new(0, 0) {
            tokio::time::sleep(delay_duration).await;
        }

        debug!("Delay completed")
    }
}
