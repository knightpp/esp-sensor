use core::convert::Infallible;

use embassy_net::{dns, tcp};
use esp_sensor::line_proto;

#[derive(Debug)]
pub(crate) enum Error {
    Dns(dns::Error),
    ConnectTcp(tcp::ConnectError),
    Tcp(tcp::Error),
    LineProto(line_proto::Error<Infallible>),
    Reqwless(reqwless::Error),
}

impl From<dns::Error> for Error {
    fn from(value: dns::Error) -> Self {
        Self::Dns(value)
    }
}

impl From<tcp::ConnectError> for Error {
    fn from(value: tcp::ConnectError) -> Self {
        Self::ConnectTcp(value)
    }
}

impl From<tcp::Error> for Error {
    fn from(value: tcp::Error) -> Self {
        Self::Tcp(value)
    }
}

impl From<line_proto::Error<Infallible>> for Error {
    fn from(value: line_proto::Error<Infallible>) -> Self {
        Self::LineProto(value)
    }
}

impl From<reqwless::Error> for Error {
    fn from(value: reqwless::Error) -> Self {
        Self::Reqwless(value)
    }
}
