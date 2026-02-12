use std::sync::Arc;

use bytes::Bytes;
use napi::bindgen_prelude::{Buffer, Uint8Array};
use napi::{Error, Result};
use napi_derive::napi;
use tokio::sync::Mutex;
use url::Url;
use web_transport_quinn::{ClientBuilder, CongestionControl, SessionError, WebTransportError};
use web_transport_quinn::{RecvStream as QuinnRecvStream, SendStream as QuinnSendStream};

#[napi(object, rename_all = "camelCase")]
pub struct ConnectOptions {
    pub server_certificate_hashes: Option<Vec<Uint8Array>>,
    pub congestion_control: Option<String>,
}

#[napi(object, rename_all = "camelCase")]
pub struct CloseInfo {
    pub close_code: u32,
    pub reason: String,
}

#[napi(object)]
pub struct BiStream {
    pub send: SendStream,
    pub recv: RecvStream,
}

#[napi]
pub struct Session {
    inner: Arc<web_transport_quinn::Session>,
}

#[napi]
impl Session {
    #[napi]
    pub async fn open_bi(&self) -> Result<BiStream> {
        let (send, recv) = self
            .inner
            .open_bi()
            .await
            .map_err(map_error)?;
        Ok(BiStream {
            send: SendStream::new(send),
            recv: RecvStream::new(recv),
        })
    }

    #[napi]
    pub async fn open_uni(&self) -> Result<SendStream> {
        let send = self.inner.open_uni().await.map_err(map_error)?;
        Ok(SendStream::new(send))
    }

    #[napi]
    pub async fn accept_bi(&self) -> Result<Option<BiStream>> {
        match self.inner.accept_bi().await {
            Ok((send, recv)) => Ok(Some(BiStream {
                send: SendStream::new(send),
                recv: RecvStream::new(recv),
            })),
            Err(err) if is_closed_error(&err) => Ok(None),
            Err(err) => Err(map_error(err)),
        }
    }

    #[napi]
    pub async fn accept_uni(&self) -> Result<Option<RecvStream>> {
        match self.inner.accept_uni().await {
            Ok(recv) => Ok(Some(RecvStream::new(recv))),
            Err(err) if is_closed_error(&err) => Ok(None),
            Err(err) => Err(map_error(err)),
        }
    }

    #[napi]
    pub async fn send_datagram(&self, payload: Uint8Array) -> Result<()> {
        let data = Bytes::from(payload.to_vec());
        self.inner.send_datagram(data).map_err(map_error)
    }

    #[napi]
    pub async fn recv_datagram(&self) -> Result<Option<Buffer>> {
        match self.inner.recv_datagram().await {
            Ok(bytes) => Ok(Some(bytes_to_buffer(bytes))),
            Err(err) if is_closed_error(&err) => Ok(None),
            Err(err) => Err(map_error(err)),
        }
    }

    #[napi]
    pub async fn max_datagram_size(&self) -> Result<u32> {
        Ok(self.inner.max_datagram_size() as u32)
    }

    #[napi]
    pub fn close(&self, code: u32, reason: String) {
        self.inner.close(code, &reason);
    }

    #[napi]
    pub async fn closed(&self) -> Result<CloseInfo> {
        let err = self.inner.closed().await;
        Ok(close_info_from_error(err))
    }
}

#[napi]
pub struct SendStream {
    inner: Arc<Mutex<QuinnSendStream>>,
}

impl SendStream {
    fn new(inner: QuinnSendStream) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

#[napi]
impl SendStream {
    #[napi]
    pub async fn write(&self, chunk: Uint8Array) -> Result<u32> {
        let mut inner = self.inner.lock().await;
        let data = chunk.to_vec();
        let written = inner.write(&data).await.map_err(map_error)?;
        Ok(written as u32)
    }

    #[napi]
    pub async fn finish(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.finish().map_err(map_error)
    }

    #[napi]
    pub fn reset(&self, code: u32) {
        let mut inner = self.inner.blocking_lock();
        inner.reset(code);
    }

    #[napi]
    pub async fn closed(&self) -> Result<Option<u32>> {
        let mut inner = self.inner.lock().await;
        let res = inner.closed().await.map_err(map_error)?;
        Ok(res.map(|code| code as u32))
    }
}

#[napi]
pub struct RecvStream {
    inner: Arc<Mutex<QuinnRecvStream>>,
}

impl RecvStream {
    fn new(inner: QuinnRecvStream) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

#[napi]
impl RecvStream {
    #[napi]
    pub async fn read(&self, max: u32) -> Result<Option<Buffer>> {
        let mut inner = self.inner.lock().await;
        match inner.read(max as usize).await.map_err(map_error)? {
            Some(bytes) => Ok(Some(bytes_to_buffer(bytes))),
            None => Ok(None),
        }
    }

    #[napi]
    pub fn stop(&self, code: u32) {
        let mut inner = self.inner.blocking_lock();
        inner.stop(code);
    }

    #[napi]
    pub async fn closed(&self) -> Result<Option<u32>> {
        let mut inner = self.inner.lock().await;
        let res = inner.closed().await.map_err(map_error)?;
        Ok(res.map(|code| code as u32))
    }
}

#[napi]
pub async fn connect(url: String, options: Option<ConnectOptions>) -> Result<Session> {
    let url = Url::parse(&url).map_err(map_error)?;

    let mut builder = ClientBuilder::new();
    if let Some(options) = &options {
        if let Some(cc) = &options.congestion_control {
            builder = builder.with_congestion_control(parse_congestion_control(cc)?);
        }
    }

    let client = if let Some(options) = options {
        if let Some(hashes) = options.server_certificate_hashes {
            let hashes = hashes.into_iter().map(|hash| hash.to_vec()).collect();
            builder.with_server_certificate_hashes(hashes).map_err(map_error)?
        } else {
            builder.with_system_roots().map_err(map_error)?
        }
    } else {
        builder.with_system_roots().map_err(map_error)?
    };

    let session = client.connect(url).await.map_err(map_error)?;

    Ok(Session {
        inner: Arc::new(session),
    })
}

fn parse_congestion_control(value: &str) -> Result<CongestionControl> {
    match value {
        "default" => Ok(CongestionControl::Default),
        "throughput" => Ok(CongestionControl::Throughput),
        "low-latency" => Ok(CongestionControl::LowLatency),
        other => Err(Error::from_reason(format!(
            "Unsupported congestion control: {other}"
        ))),
    }
}

fn is_closed_error(err: &SessionError) -> bool {
    match err {
        SessionError::WebTransportError(WebTransportError::Closed(_, _)) => true,
        SessionError::ConnectionError(_) => true,
        _ => false,
    }
}

fn close_info_from_error(err: SessionError) -> CloseInfo {
    match err {
        SessionError::WebTransportError(WebTransportError::Closed(code, reason)) => CloseInfo {
            close_code: code,
            reason,
        },
        other => CloseInfo {
            close_code: 0,
            reason: other.to_string(),
        },
    }
}

fn bytes_to_buffer(bytes: Bytes) -> Buffer {
    Buffer::from(bytes.to_vec())
}

fn map_error<E: std::fmt::Display>(err: E) -> Error {
    Error::from_reason(err.to_string())
}
