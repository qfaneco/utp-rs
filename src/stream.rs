use std::{
    io::Result,
    sync::Arc,
    pin::Pin,
    future::Future,
    net::SocketAddr,
    task::{Poll, Context},
    cmp::min
};

use tokio::{
    net::ToSocketAddrs,
    sync::RwLock,
    io::{AsyncRead, ReadBuf, AsyncWrite}
};

use super::socket::UtpSocket;

/// A structure that represents a uTP (Micro Transport Protocol) stream between a local socket and a
/// remote socket.
///
/// The connection will be closed when the value is dropped (either explicitly or when it goes out
/// of scope).
///
/// The default maximum retransmission retries is 5, which translates to about 16 seconds. It can be
/// changed by calling `set_max_retransmission_retries`. Notice that the initial congestion timeout
/// is 500 ms and doubles with each timeout.
#[derive(Clone)]
pub struct UtpStream {
    socket: Arc<RwLock<UtpSocket>>,
    futures: Arc<UtpStreamFutures>,
}

unsafe impl Send for UtpStream {}
type OptionIoFuture<T> = RwLock<Option<Pin<Box<dyn Future<Output = Result<T>> + Send + 'static>>>>;

#[derive(Default)]
struct UtpStreamFutures {
    read: OptionIoFuture<(Vec<u8>, usize)>,
    write: OptionIoFuture<usize>,
    flush: OptionIoFuture<()>,
    close: OptionIoFuture<()>,
}

impl UtpStream {
    /// Creates a uTP stream listening on the given address.
    ///
    /// The address type can be any implementer of the `ToSocketAddr` trait. See its documentation
    /// for concrete examples.
    ///
    /// If more than one valid address is specified, only the first will be used.
    pub async fn bind<A: ToSocketAddrs>(addr: A) -> Result<UtpStream> {
        let socket = UtpSocket::bind(addr).await?;
        Ok(UtpStream {
            socket: Arc::new(RwLock::new(socket)),
            futures: UtpStreamFutures::default().into(),
        })
    }

    /// Opens a uTP connection to a remote host by hostname or IP address.
    ///
    /// The address type can be any implementer of the `ToSocketAddr` trait. See its documentation
    /// for concrete examples.
    ///
    /// If more than one valid address is specified, only the first will be used.
    pub async fn connect<A: ToSocketAddrs>(dst: A) -> Result<UtpStream> {
        // Port 0 means the operating system gets to choose it
        let socket = UtpSocket::connect(dst).await?;
        Ok(UtpStream {
            socket: Arc::new(RwLock::new(socket)),
            futures: UtpStreamFutures::default().into(),
        })
    }

    /// Gracefully closes connection to peer.
    ///
    /// This method allows both peers to receive all packets still in
    /// flight.
    pub async fn close(&mut self) -> Result<()> {
        self.socket.write().await.close().await
    }

    /// Returns the socket address of the local half of this uTP connection.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.socket.try_read().unwrap().local_addr()
    }

    /// Returns the socket address of the remote half of this uTP connection.
    pub fn peer_addr(&self) -> Result<SocketAddr> {
        self.socket.try_read().unwrap().peer_addr()
    }

    /// Changes the maximum number of retransmission retries on the underlying socket.
    pub async fn set_max_retransmission_retries(&mut self, n: u32) {
        self.socket.write().await.max_retransmission_retries = n;
    }
}

impl AsyncRead for UtpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<()>> {
        if self.futures.read.try_read().unwrap().is_none() {
            let socket = self.socket.clone();
            let chunk = min(1380, buf.remaining());
            let mut vec = Vec::from(buf.initialize_unfilled_to(chunk));

            *self.futures.read.try_write().unwrap() = Some(Box::pin(async move {
                let (nread, _) = socket.write().await.recv_from(&mut vec).await?;
                Ok((vec, nread))
            }));
        }

        let (bytes, n) = {
            let mut fut = self.futures.read.try_write().unwrap();
            match Pin::new(fut.as_mut().unwrap()).poll(cx) {
                Poll::Ready(res) => res?,
                Poll::Pending => return Poll::Pending,
            }
        };

        if n > 0 {
            buf.put_slice(&bytes[..]);
        }
        
        *self.futures.read.try_write().unwrap() = None;
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for UtpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize>> {
        if self.futures.write.try_read().unwrap().is_none() {
            let socket = self.socket.clone();
            let vec = Vec::from(buf);
            *self.futures.write.try_write().unwrap() = Some(Box::pin(
                async move { socket.write().await.send_to(&vec).await }
            ));
        }

        let n = {
            let mut fut = self.futures.write.try_write().unwrap();
            match Pin::new(fut.as_mut().unwrap()).poll(cx) {
                Poll::Ready(res) => res?,
                Poll::Pending => return Poll::Pending,
            }
        };
        *self.futures.write.try_write().unwrap() = None;
        Poll::Ready(Ok(n))
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<()>> {
        if self.futures.flush.try_read().unwrap().is_none() {
            let socket = self.socket.clone();
            *self.futures.flush.try_write().unwrap() = Some(Box::pin(
                async move { socket.write().await.flush().await }
            ));
        }

        let result = {
            let mut fut = self.futures.flush.try_write().unwrap();
            match Pin::new(fut.as_mut().unwrap()).poll(cx) {
                Poll::Ready(res) => res,
                Poll::Pending => return Poll::Pending,
            }
        };
        *self.futures.flush.try_write().unwrap() = None;
        Poll::Ready(result)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<()>> {
        if self.futures.close.try_read().unwrap().is_none() {
            let socket = self.socket.clone();
            *self.futures.close.try_write().unwrap() = Some(Box::pin(
                async move { socket.write().await.flush().await }
            ));
        }

        let result = {
            let mut fut = self.futures.close.try_write().unwrap();
            match Pin::new(fut.as_mut().unwrap()).poll(cx) {
                Poll::Ready(res) => res,
                Poll::Pending => return Poll::Pending,
            }
        };
        *self.futures.close.try_write().unwrap() = None;
        Poll::Ready(result)
    }
}

impl From<UtpSocket> for UtpStream {
    fn from(socket: UtpSocket) -> Self {
        UtpStream {
            socket: Arc::new(RwLock::new(socket)),
            futures: UtpStreamFutures::default().into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use crate::tests::{next_test_ip4, next_test_ip6};
    use super::UtpStream;

    macro_rules! iotry {
        ($e:expr) => {
            match $e.await {
                Ok(e) => e,
                Err(e) => panic!("{}", e),
            }
        };
    }

    #[tokio::test]
    async fn test_stream_open_and_close() {
        let server_addr = next_test_ip4();
        let mut server = iotry!(UtpStream::bind(server_addr));

        let child = tokio::spawn(async move {
            let mut client = iotry!(UtpStream::connect(server_addr));
            iotry!(client.close());
            drop(client);
        });

        let mut received = vec![];
        iotry!(server.read_to_end(&mut received));
        iotry!(server.close());
        child.await.unwrap();
    }

    #[tokio::test]
    async fn test_stream_open_and_close_ipv6() {
        let server_addr = next_test_ip6();
        let mut server = iotry!(UtpStream::bind(server_addr));

        let child = tokio::spawn(async move {
            let mut client = iotry!(UtpStream::connect(server_addr));
            iotry!(client.close());
            drop(client);
        });

        let mut received = vec![];
        iotry!(server.read_to_end(&mut received));
        iotry!(server.close());
        child.await.unwrap();
    }

    #[tokio::test]
    async fn test_stream_small_data() {
        // Fits in a packet
        const LEN: usize = 1024;
        let data: Vec<u8> = (0..LEN).map(|idx| idx as u8).collect();
        assert_eq!(LEN, data.len());

        let d = data.clone();
        let server_addr = next_test_ip4();
        let mut server = iotry!(UtpStream::bind(server_addr));

        let child = tokio::spawn(async move {
            let mut client = iotry!(UtpStream::connect(server_addr));
            iotry!(client.write(&d[..]));
            iotry!(client.close());
        });

        let mut received = Vec::with_capacity(LEN);
        iotry!(server.read_to_end(&mut received));
        assert!(!received.is_empty());
        assert_eq!(received.len(), data.len());
        assert_eq!(received, data);
        child.await.unwrap();
    }

    #[tokio::test]
    async fn test_stream_large_data() {
        // Has to be sent over several packets
        const LEN: usize = 1024 * 1024;
        let data: Vec<u8> = (0..LEN).map(|idx| idx as u8).collect();
        assert_eq!(LEN, data.len());

        let d = data.clone();
        let server_addr = next_test_ip4();
        let mut server = iotry!(UtpStream::bind(server_addr));

        let child = tokio::spawn(async move {
            let mut client = iotry!(UtpStream::connect(server_addr));
            iotry!(client.write(&d[..]));
            iotry!(client.close());
        });

        let mut received = Vec::with_capacity(LEN);
        iotry!(server.read_to_end(&mut received));
        assert!(!received.is_empty());
        assert_eq!(received.len(), data.len());
        assert_eq!(received, data);
        child.await.unwrap();
    }

    #[tokio::test]
    async fn test_stream_successive_reads() {
        const LEN: usize = 1024;
        let data: Vec<u8> = (0..LEN).map(|idx| idx as u8).collect();
        assert_eq!(LEN, data.len());

        let d = data.clone();
        let server_addr = next_test_ip4();
        let mut server = iotry!(UtpStream::bind(server_addr));

        let child = tokio::spawn(async move {
            let mut client = iotry!(UtpStream::connect(server_addr));
            iotry!(client.write(&d[..]));
            iotry!(client.close());
        });

        let mut received = Vec::with_capacity(LEN);
        iotry!(server.read_to_end(&mut received));
        assert!(!received.is_empty());
        assert_eq!(received.len(), data.len());
        assert_eq!(received, data);

        assert_eq!(server.read(&mut received).await.unwrap(), 0);
        child.await.unwrap();
    }

    #[tokio::test]
    async fn test_local_addr() {
        use std::net::ToSocketAddrs;

        let addr = next_test_ip4();
        let addr = addr.to_socket_addrs().unwrap().next().unwrap();
        let stream = UtpStream::bind(addr).await.unwrap();

        assert!(stream.local_addr().is_ok());
        assert_eq!(stream.local_addr().unwrap(), addr);
    }

    #[tokio::test]
    async fn test_clone() {
        use std::net::ToSocketAddrs;

        let addr = next_test_ip4();
        let addr = addr.to_socket_addrs().unwrap().next().unwrap();
        let response = tokio::spawn(async move {
            let mut server = UtpStream::bind(addr).await.unwrap();
            let mut response = Vec::with_capacity(6);

            let mut buf = vec![0; 3];
            server.read(&mut buf).await.unwrap();
            response.append(&mut buf);

            let mut buf = vec![0; 3];
            server.read(&mut buf).await.unwrap();
            response.append(&mut buf);

            response
        });

        let mut client1 = UtpStream::connect(addr).await.unwrap();
        let mut client2 = client1.clone();

        tokio::spawn(async move {
            client1.write(&[0, 1, 2]).await.unwrap();
        })
        .await.unwrap();

        tokio::spawn(async move {
            client2.write(&[3, 4, 5]).await.unwrap();
        })
        .await.unwrap();

        let result = response.await.unwrap();
        assert_eq!(result, vec![0, 1, 2, 3, 4, 5]);
    }
}
