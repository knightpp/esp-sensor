use core::mem::ManuallyDrop;

use alloc::boxed::Box;
use embassy_net::{driver::Driver, tcp::TcpSocket, Stack};

#[derive(Debug)]
pub enum ConnectorError {
    Tcp(embassy_net::tcp::Error),
    TcpConnect(embassy_net::tcp::ConnectError),
}

impl embedded_svc::io::Error for ConnectorError {
    fn kind(&self) -> embedded_svc::io::ErrorKind {
        match self {
            Self::Tcp(err) => err.kind(),
            Self::TcpConnect(err) => err.kind(),
        }
    }
}

impl From<embassy_net::tcp::Error> for ConnectorError {
    fn from(value: embassy_net::tcp::Error) -> Self {
        Self::Tcp(value)
    }
}

impl From<embassy_net::tcp::ConnectError> for ConnectorError {
    fn from(value: embassy_net::tcp::ConnectError) -> Self {
        Self::TcpConnect(value)
    }
}

pub struct TcpConnect<'s, D: Driver> {
    stack: &'s Stack<D>,
}

impl<'s, D: Driver> TcpConnect<'s, D> {
    pub const fn new(stack: &'s Stack<D>) -> Self {
        Self { stack }
    }
}

impl<'s, D: Driver> embedded_nal_async::TcpConnect for TcpConnect<'s, D> {
    type Error = ConnectorError;

    type Connection<'a> = TcpSocketBuffers<'a , 1500> where Self: 'a;

    async fn connect<'a>(
        &'a self,
        remote: embedded_nal_async::SocketAddr,
    ) -> Result<Self::Connection<'a>, Self::Error>
    where
        Self: 'a,
    {
        let mut conn = Self::Connection::new(self.stack);
        conn.connect(remote).await?;
        Ok(conn)
    }
}

pub struct TcpSocketBuffers<'a, const N: usize> {
    // TODO: futures cannot be sent between threads because this is not send
    inner: Option<TcpSocket<'a>>,
    // buffers should be after inner field to guarantee drop ordering
    rx_buffer: ManuallyDrop<Box<[u8; N]>>,
    tx_buffer: ManuallyDrop<Box<[u8; N]>>,
}

impl<'a, const N: usize> Drop for TcpSocketBuffers<'a, N> {
    fn drop(&mut self) {
        unsafe {
            ManuallyDrop::drop(&mut self.rx_buffer);
            ManuallyDrop::drop(&mut self.tx_buffer);
        }
    }
}

impl<'a, const N: usize> TcpSocketBuffers<'a, N> {
    fn new<D: Driver>(stack: &'a Stack<D>) -> Self {
        let mut this = Self {
            inner: None,
            rx_buffer: ManuallyDrop::new(Box::new([0; N])),
            tx_buffer: ManuallyDrop::new(Box::new([0; N])),
        };

        // TODO: verify correctness
        let mut socket = TcpSocket::new(
            stack,
            unsafe { core::mem::transmute(&mut this.rx_buffer[..]) },
            unsafe { core::mem::transmute(&mut this.tx_buffer[..]) },
        );
        socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

        this.inner = Some(socket);
        this
    }

    async fn connect(
        &mut self,
        remote: embedded_nal_async::SocketAddr,
    ) -> Result<(), embassy_net::tcp::ConnectError> {
        let (address, port) = {
            match remote {
                embedded_nal_async::SocketAddr::V4(v4) => (
                    smoltcp::wire::IpAddress::Ipv4(smoltcp::wire::Ipv4Address(v4.ip().octets())),
                    v4.port(),
                ),
                embedded_nal_async::SocketAddr::V6(_v6) => unreachable!(),
            }
        };
        let remote = smoltcp::wire::IpEndpoint::new(address, port);
        unsafe { self.inner.as_mut().unwrap_unchecked() }
            .connect(remote)
            .await
    }
}

impl<'a, const N: usize> embedded_svc::io::Io for TcpSocketBuffers<'a, N> {
    type Error = ConnectorError;
}

impl<'a, const N: usize> embedded_svc::io::asynch::Write for TcpSocketBuffers<'a, N> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let n = unsafe { self.inner.as_mut().unwrap_unchecked() }
            .write(buf)
            .await?;
        Ok(n)
    }
}

impl<'a, const N: usize> embedded_svc::io::asynch::Read for TcpSocketBuffers<'a, N> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let n = unsafe { self.inner.as_mut().unwrap_unchecked() }
            .read(buf)
            .await?;
        Ok(n)
    }
}

pub struct Dns<'s, D: Driver> {
    // TODO: futures cannot be sent between threads because this is not send
    stack: &'s Stack<D>,
}

impl<'s, D: Driver> Dns<'s, D> {
    pub const fn new(stack: &'s Stack<D>) -> Self {
        Self { stack }
    }
}

impl<'s, D: Driver + 'static> embedded_nal_async::Dns for Dns<'s, D> {
    type Error = embassy_net::dns::Error;

    async fn get_host_by_name(
        &self,
        host: &str,
        addr_type: embedded_nal_async::AddrType,
    ) -> Result<embedded_nal_async::IpAddr, Self::Error> {
        use embedded_nal_async::AddrType;
        let qtype = match addr_type {
            AddrType::IPv4 | AddrType::Either => embassy_net::dns::DnsQueryType::A,
            AddrType::IPv6 => embassy_net::dns::DnsQueryType::Aaaa,
        };

        let result = self.stack.dns_query(host, qtype).await?;

        Ok(match result.first().unwrap() {
            embassy_net::IpAddress::Ipv4(v4) => {
                embedded_nal_async::IpAddr::V4(embedded_nal_async::Ipv4Addr::from(v4.0))
            }
        })
    }

    #[allow(clippy::unused_async)]
    async fn get_host_by_address(
        &self,
        _addr: embedded_nal_async::IpAddr,
    ) -> Result<embedded_nal_async::heapless::String<256>, Self::Error> {
        unimplemented!()
    }
}