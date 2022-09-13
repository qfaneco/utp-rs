use std::{
    io::Result,
    sync::Arc,
    net::SocketAddr,
    pin::Pin,
    future::Future,
    task::{Poll, Context},
    async_iter::AsyncIterator,
};

use tokio::net::{UdpSocket, ToSocketAddrs};

use super::{
    socket::{UtpSocket, BUF_SIZE},
    packet::{Packet, PacketType},
    error::SocketError
};

/// A structure representing a socket server.
#[derive(Clone)]
pub struct UtpListener {
    /// The public facing UDP socket
    socket: Arc<UdpSocket>,
}

impl UtpListener {
    /// Creates a new `UtpListener` bound to a specific address.
    ///
    /// The resulting listener is ready for accepting connections.
    ///
    /// The address type can be any implementer of the `ToSocketAddr` trait. See its documentation
    /// for concrete examples.
    ///
    /// If more than one valid address is specified, only the first will be used.
    pub async fn bind<A: ToSocketAddrs>(addr: A) -> Result<UtpListener> {
        let socket = UdpSocket::bind(addr).await?;
        Ok(UtpListener {
            socket: socket.into(),
        })
    }

    /// Accepts a new incoming connection from this listener.
    ///
    /// This function will block the caller until a new uTP connection is established. When
    /// established, the corresponding `UtpSocket` and the peer's remote address will be returned.
    ///
    /// Notice that the resulting `UtpSocket` is bound to a different local port than the public
    /// listening port (which `UtpListener` holds). This may confuse the remote peer!
    pub async fn accept(&self) -> Result<(UtpSocket, SocketAddr)> {
        let mut buf = vec![0; BUF_SIZE];

        let (nread, src) = self.socket.recv_from(&mut buf).await?;
        let packet = Packet::try_from(&buf[..nread])?;

        // Ignore non-SYN packets
        if packet.get_type() != PacketType::Syn {
            let message = format!("Expected SYN packet, got {:?} instead", packet.get_type());
            return Err(SocketError::Other(message).into());
        }

        // The address of the new socket will depend on the type of the listener.
        let local_addr = self.socket.local_addr()?;
        let inner_socket = match local_addr {
            SocketAddr::V4(_) => UdpSocket::bind("0.0.0.0:0"),
            SocketAddr::V6(_) => UdpSocket::bind("[::]:0"),
        }
        .await?;

        let mut socket = UtpSocket::from_raw_parts(inner_socket, src);

        // Establish connection with remote peer
        if let Ok(Some(reply)) = socket.handle_packet(&packet, src).await {
            socket
                .socket
                .send_to(reply.as_ref(), src)
                .await
                .and(Ok((socket, src)))
        } else {
            Err(SocketError::Other("Reached unreachable statement".to_owned()).into())
        }
    }

    /// Returns an iterator over the connections being received by this listener.
    ///
    /// The returned iterator will never return `None`.
    pub fn incoming(&self) -> Incoming<'_> {
        Incoming {
            listener: self,
            accept: None,
        }
    }

    /// Returns the local socket address of this listener.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.socket.local_addr()
    }
}

type AcceptFuture<'a> = Option<Pin<Box<dyn Future<Output = Result<(UtpSocket, SocketAddr)>> + Send + 'a>>>;


pub struct Incoming<'a> {
    listener: &'a UtpListener,
    accept: AcceptFuture<'a>,
}

impl<'a> AsyncIterator for Incoming<'a> {
    type Item = Result<(UtpSocket, SocketAddr)>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        loop {
            if self.accept.is_none() {
                self.accept = Some(Box::pin(self.listener.accept()));
            }

            if let Some(f) = &mut self.accept {
                let res = match f.as_mut().poll(cx) {
                    Poll::Ready(res) => res,
                    Poll::Pending => return Poll::Pending,
                };
                self.accept = None;
                return Poll::Ready(Some(res));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::ToSocketAddrs;
    use crate::{tests::next_test_ip4, socket::UtpSocket};
    use super::UtpListener;

    #[tokio::test]
    async fn test_listener_local_addr() {
        let addr = next_test_ip4();
        let addr = addr.to_socket_addrs().unwrap().next().unwrap();
        let listener = UtpListener::bind(addr).await.unwrap();

        assert!(listener.local_addr().is_ok());
        assert_eq!(listener.local_addr().unwrap(), addr);
    }

    #[tokio::test]
    async fn test_listener_listener_clone() {
        let addr = next_test_ip4();
        let addr = addr.to_socket_addrs().unwrap().next().unwrap();

        // setup listener and clone to be used on two tasks
        let listener1 = UtpListener::bind(addr).await.unwrap();
        let listener2 = listener1.clone();

        tokio::spawn(async move { listener1.accept().await.unwrap() });
        tokio::spawn(async move { listener2.accept().await.unwrap() });

        // Connect twice - to each listerner
        tokio::spawn(async move {
            UtpSocket::connect(addr).await.unwrap();
            UtpSocket::connect(addr).await.unwrap();
        })
        .await
        .unwrap();
    }
}
