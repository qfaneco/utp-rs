use std::{
    error::Error,
    fmt,
    io::{self, ErrorKind}
};

#[derive(Debug)]
pub enum SocketError {
    ConnectionClosed,
    ConnectionReset,
    ConnectionTimedOut,
    InvalidAddress,
    InvalidReply,
    NotConnected,
    Other(String),
}

impl fmt::Display for SocketError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::SocketError::*;
        match *self {
            ConnectionClosed   => write!(f, "The socket is closed"),
            ConnectionReset    => write!(f, "Connection reset by remote peer"),
            ConnectionTimedOut => write!(f, "Connection timed out"),
            InvalidAddress     => write!(f, "Invalid address"),
            InvalidReply       => write!(f, "The remote peer sent an invalid reply"),
            NotConnected       => write!(f, "The socket is not connected"),
            Other(ref s)       => write!(f, "{}", s),
        }
    }
}

impl Error for SocketError {}

impl From<SocketError> for io::Error {
    fn from(error: SocketError) -> io::Error {
        use self::SocketError::*;
        let kind = match error {
            ConnectionClosed |
            NotConnected       => ErrorKind::NotConnected,
            ConnectionReset    => ErrorKind::ConnectionReset,
            ConnectionTimedOut => ErrorKind::TimedOut,
            InvalidAddress     => ErrorKind::InvalidInput,
            InvalidReply       => ErrorKind::ConnectionRefused,
            Other(_)           => ErrorKind::Other,
        };
        io::Error::new(kind, error)
    }
}

#[derive(Debug)]
pub enum ParseError {
    InvalidExtensionLength,
    InvalidPacketLength,
    InvalidPacketType(u8),
    UnsupportedVersion,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::ParseError::*;
        match *self {
            InvalidExtensionLength  => write!(f, "Invalid extension length (must be a non-zero multiple of 4)"),
            InvalidPacketLength     => write!(f, "The packet is too small"),
            InvalidPacketType(_)    => write!(f, "Invalid packet type"),
            UnsupportedVersion      => write!(f, "Unsupported packet version"),
        }
    }
}

impl Error for ParseError {}

impl From<ParseError> for io::Error {
    fn from(error: ParseError) -> io::Error {
        io::Error::new(ErrorKind::Other, error)
    }
}
