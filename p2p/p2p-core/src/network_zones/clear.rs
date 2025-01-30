use std::{
    net::{IpAddr, SocketAddr},
    pin::Pin,
    task::{Context, Poll},
};
use std::net::Ipv4Addr;
use std::str::FromStr;
use futures::Stream;
use tokio::net::{
    tcp::{OwnedReadHalf, OwnedWriteHalf},
    TcpListener, TcpStream,
};
use tokio_util::codec::{FramedRead, FramedWrite};

use cuprate_wire::MoneroWireCodec;

use crate::{NetZoneAddress, NetworkZone};

impl NetZoneAddress for SocketAddr {
    type BanID = IpAddr;

    fn set_port(&mut self, port: u16) {
        Self::set_port(self, port);
    }

    fn ban_id(&self) -> Self::BanID {
        self.ip()
    }

    fn make_canonical(&mut self) {
        let ip = self.ip().to_canonical();
        self.set_ip(ip);
    }

    fn should_add_to_peer_list(&self) -> bool {
        // TODO
        true
    }

    fn read_ban_line(s: &str) -> Vec<Self> {
        let mut ips = Vec::new();

        let mut s = s.split('/');
        let ip = Ipv4Addr::from_str(s.next().unwrap()).unwrap();
        if let Some(_) = s.next() {
            let mut oct = ip.octets();
            for i in 0..u8::MAX {
                oct[3] = i;
                ips.push(SocketAddr::new(Ipv4Addr::from(oct).into(), 0));
            }
        } else {
            ips.push(SocketAddr::new(Ipv4Addr::from(ip).into(), 0));
        }

        ips

    }
}

#[derive(Debug, Clone)]
pub struct ClearNetServerCfg {
    pub ip: IpAddr,
}

#[derive(Clone, Copy)]
pub enum ClearNet {}

#[async_trait::async_trait]
impl NetworkZone for ClearNet {
    const NAME: &'static str = "ClearNet";

    const CHECK_NODE_ID: bool = true;

    type Addr = SocketAddr;
    type Stream = FramedRead<OwnedReadHalf, MoneroWireCodec>;
    type Sink = FramedWrite<OwnedWriteHalf, MoneroWireCodec>;
    type Listener = InBoundStream;

    type ServerCfg = ClearNetServerCfg;

    async fn connect_to_peer(
        addr: Self::Addr,
    ) -> Result<(Self::Stream, Self::Sink), std::io::Error> {
        let (read, write) = TcpStream::connect(addr).await?.into_split();
        Ok((
            FramedRead::new(read, MoneroWireCodec::default()),
            FramedWrite::new(write, MoneroWireCodec::default()),
        ))
    }

    async fn incoming_connection_listener(
        config: Self::ServerCfg,
        port: u16,
    ) -> Result<Self::Listener, std::io::Error> {
        let listener = TcpListener::bind(SocketAddr::new(config.ip, port)).await?;
        Ok(InBoundStream { listener })
    }
}

pub struct InBoundStream {
    listener: TcpListener,
}

impl Stream for InBoundStream {
    type Item = Result<
        (
            Option<SocketAddr>,
            FramedRead<OwnedReadHalf, MoneroWireCodec>,
            FramedWrite<OwnedWriteHalf, MoneroWireCodec>,
        ),
        std::io::Error,
    >;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.listener
            .poll_accept(cx)
            .map_ok(|(stream, mut addr)| {
                let ip = addr.ip().to_canonical();
                addr.set_ip(ip);

                let (read, write) = stream.into_split();
                (
                    Some(addr),
                    FramedRead::new(read, MoneroWireCodec::default()),
                    FramedWrite::new(write, MoneroWireCodec::default()),
                )
            })
            .map(Some)
    }
}
