use alloc::{format, string::String};
use embedded_svc::io::asynch::{Read, Write};
use reqwless::{
    client::HttpResource, headers::ContentType, request::RequestBuilder, response::Status,
};

#[derive(Debug)]
pub enum Error {
    Reqwless(reqwless::Error),
    Utf8(core::str::Utf8Error),
    Api,
}

impl From<reqwless::Error> for Error {
    fn from(value: reqwless::Error) -> Self {
        Self::Reqwless(value)
    }
}

impl From<core::str::Utf8Error> for Error {
    fn from(value: core::str::Utf8Error) -> Self {
        Self::Utf8(value)
    }
}

pub struct Client<'r, C: Read + Write> {
    resource: HttpResource<'r, C>,
    token: String,
}

impl<'r, C: Read + Write> Client<'r, C> {
    pub fn new(resource: HttpResource<'r, C>, token: &str) -> Self {
        Self {
            resource,
            token: format!("Token {token}"),
        }
    }

    pub async fn write<'s>(&mut self, org: &str, bucket: &str, body: &[u8]) -> Result<(), Error> {
        let mut buf = [0u8; 1024];
        let headers = [
            ("Authorization", self.token.as_str()),
            ("Accept", "application/json"),
        ];
        let path = format!("/api/v2/write?org={org}&bucket={bucket}&precision=ns");

        let response = self
            .resource
            // SAFETY: use request before dropping path
            .post(unsafe { core::mem::transmute(path.as_str()) })
            .body(body)
            .content_type(ContentType::TextPlain)
            .headers(&headers)
            .send(&mut buf)
            .await?;

        match &response.status {
            Status::Ok | Status::Created | Status::Accepted => Ok(()),
            _ => {
                let resp_body = response.body()?.read_to_end().await?;
                let resp_body = core::str::from_utf8(resp_body)?;
                log::error!("failed request: response={resp_body}");
                Err(Error::Api)
            }
        }
    }
}
