use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use governor::clock::DefaultClock;
use governor::state::keyed::DashMapStateStore;
use governor::{Quota, RateLimiter};
use pyo3::exceptions::{PyStopAsyncIteration, PyValueError};
use pyo3::prelude::*;
use tokio::sync::Mutex;

use crate::errors;
use crate::runtime;
use crate::session::Session;

type IpRateLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;

type Handshakes = FuturesUnordered<
    BoxFuture<'static, Result<web_transport_quinn::Request, web_transport_quinn::ServerError>>,
>;

#[pyclass]
pub struct Server {
    endpoint: quinn::Endpoint,
    handshakes: Arc<Mutex<Handshakes>>,
    local_addr: (String, u16),
    transport_config: Arc<quinn::TransportConfig>,
    rate_limiter: Option<Arc<IpRateLimiter>>,
    // Epoch seconds of last `retain_recent()` call (0 = never).
    last_cleanup: Arc<AtomicU64>,
}

#[pymethods]
impl Server {
    #[new]
    #[pyo3(signature = (*, certificate_chain, private_key, bind="[::]:4433", congestion_control="default", max_idle_timeout=Some(30.0), keep_alive_interval=None, rate_limit_per_ip=None, rate_limit_max_burst=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        certificate_chain: Vec<Vec<u8>>,
        private_key: Vec<u8>,
        bind: &str,
        congestion_control: &str,
        max_idle_timeout: Option<f64>,
        keep_alive_interval: Option<f64>,
        rate_limit_per_ip: Option<f64>,
        rate_limit_max_burst: Option<u32>,
    ) -> PyResult<Self> {
        let addr: SocketAddr = bind
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid bind address: {e}")))?;

        let tls_config = build_tls_config(certificate_chain, private_key)?;

        // Build transport config
        let mut transport = quinn::TransportConfig::default();
        transport.max_idle_timeout(
            max_idle_timeout
                .map(Duration::try_from_secs_f64)
                .transpose()
                .map_err(|_| PyValueError::new_err("invalid max_idle_timeout"))?
                .map(quinn::IdleTimeout::try_from)
                .transpose()
                .map_err(|e| PyValueError::new_err(format!("invalid idle timeout: {e}")))?,
        );
        transport.keep_alive_interval(
            keep_alive_interval
                .map(Duration::try_from_secs_f64)
                .transpose()
                .map_err(|_| PyValueError::new_err("invalid keep_alive_interval"))?,
        );

        // Congestion control — matches ClientBuilder::with_congestion_control()
        let congestion_controller: Option<
            Arc<dyn quinn::congestion::ControllerFactory + Send + Sync + 'static>,
        > = match congestion_control {
            "default" => None,
            "throughput" => Some(Arc::new(quinn::congestion::CubicConfig::default())),
            "low_latency" => Some(Arc::new(quinn::congestion::BbrConfig::default())),
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown congestion control: {other}"
                )));
            }
        };

        if let Some(cc) = congestion_controller {
            transport.congestion_controller_factory(cc);
        }
        let transport_config = Arc::new(transport);

        // Build quinn server config
        let quic_config = quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
            .map_err(|e| PyValueError::new_err(format!("QUIC config error: {e}")))?;
        let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_config));
        server_config.transport_config(transport_config.clone());

        // Bind endpoint
        let _guard = runtime::get_runtime().enter();
        let endpoint = quinn::Endpoint::server(server_config, addr)
            .map_err(|e| PyValueError::new_err(format!("failed to bind: {e}")))?;

        let local_addr = endpoint
            .local_addr()
            .map_err(|e| PyValueError::new_err(format!("failed to get local addr: {e}")))?;

        // Build rate limiter
        let rate_limiter = build_rate_limiter(rate_limit_per_ip, rate_limit_max_burst)?;

        Ok(Self {
            endpoint,
            handshakes: Arc::new(Mutex::new(FuturesUnordered::new())),
            local_addr: (local_addr.ip().to_string(), local_addr.port()),
            transport_config,
            rate_limiter,
            last_cleanup: Arc::new(AtomicU64::new(0)),
        })
    }

    fn __aenter__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let obj: Py<PyAny> = slf.into_pyobject(py)?.into_any().unbind();
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(obj) })
    }

    #[pyo3(signature = (_exc_type=None, _exc_val=None, _exc_tb=None))]
    fn __aexit__<'py>(
        &mut self,
        py: Python<'py>,
        _exc_type: Option<Py<PyAny>>,
        _exc_val: Option<Py<PyAny>>,
        _exc_tb: Option<Py<PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        // Close and wait for idle
        self.endpoint.close(quinn::VarInt::from_u32(0), b"");
        let endpoint = self.endpoint.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            endpoint.wait_idle().await;
            Ok(())
        })
    }

    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let endpoint = self.endpoint.clone();
        let handshakes = self.handshakes.clone();
        let rate_limiter = self.rate_limiter.clone();
        let last_cleanup = self.last_cleanup.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            match accept_inner(&endpoint, &handshakes, &rate_limiter, &last_cleanup).await {
                Some(request) => Ok(SessionRequest::new(request)),
                None => Err(PyStopAsyncIteration::new_err(())),
            }
        })
    }

    /// Wait for the next incoming session request.
    fn accept<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let endpoint = self.endpoint.clone();
        let handshakes = self.handshakes.clone();
        let rate_limiter = self.rate_limiter.clone();
        let last_cleanup = self.last_cleanup.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Ok(
                accept_inner(&endpoint, &handshakes, &rate_limiter, &last_cleanup)
                    .await
                    .map(SessionRequest::new),
            )
        })
    }

    /// Close all connections immediately.
    #[pyo3(signature = (code=0, reason=""))]
    fn close(&self, code: u64, reason: &str) -> PyResult<()> {
        let var_code = quinn::VarInt::from_u64(code)
            .map_err(|_| PyValueError::new_err("code must be less than 2**62"))?;
        self.endpoint.close(var_code, reason.as_bytes());
        Ok(())
    }

    /// Wait for all connections to be cleanly shut down.
    fn wait_closed<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let endpoint = self.endpoint.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            endpoint.wait_idle().await;
            Ok(())
        })
    }

    /// The local ``(host, port)`` the server is bound to.
    #[getter]
    fn local_addr(&self) -> (String, u16) {
        self.local_addr.clone()
    }

    /// Replace the TLS certificate for new incoming connections.
    fn reload_certificates(
        &self,
        certificate_chain: Vec<Vec<u8>>,
        private_key: Vec<u8>,
    ) -> PyResult<()> {
        let tls_config = build_tls_config(certificate_chain, private_key)?;
        let quic_config = quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
            .map_err(|e| PyValueError::new_err(format!("QUIC config error: {e}")))?;
        let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_config));
        server_config.transport_config(self.transport_config.clone());
        self.endpoint.set_server_config(Some(server_config));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Accept helper
// ---------------------------------------------------------------------------

/// Accept the next session request, auto-refusing rate-limited connections
/// at the QUIC level (before TLS handshake).
async fn accept_inner(
    endpoint: &quinn::Endpoint,
    handshakes: &Mutex<Handshakes>,
    rate_limiter: &Option<Arc<IpRateLimiter>>,
    last_cleanup: &AtomicU64,
) -> Option<web_transport_quinn::Request> {
    let mut handshakes = handshakes.lock().await;
    loop {
        tokio::select! {
            res = endpoint.accept() => {
                let incoming = res?;

                // Rate limiting gate (pre-TLS)
                if let Some(ref limiter) = rate_limiter {
                    // Periodically evict stale IP entries (at most once per 60s)
                    maybe_cleanup(limiter, last_cleanup);

                    // Force address validation if not already validated
                    if !incoming.remote_address_validated() && incoming.may_retry() {
                        let _ = incoming.retry();
                        continue;
                    }
                    // Check rate limit for this IP
                    if limiter.check_key(&incoming.remote_address().ip()).is_err() {
                        incoming.refuse();
                        continue;
                    }
                }

                // Proceed with TLS + H3 handshake (concurrent)
                handshakes.push(Box::pin(async move {
                    let conn = incoming.await?;
                    web_transport_quinn::Request::accept(conn).await
                }));
            }
            Some(res) = handshakes.next() => {
                if let Ok(request) = res {
                    return Some(request);
                }
            }
        }
    }
}

const CLEANUP_INTERVAL_SECS: u64 = 10;

/// Evict stale entries from the rate limiter at most once per `CLEANUP_INTERVAL_SECS`.
fn maybe_cleanup(limiter: &IpRateLimiter, last_cleanup: &AtomicU64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let prev = last_cleanup.load(Ordering::Relaxed);
    if now.saturating_sub(prev) >= CLEANUP_INTERVAL_SECS
        && last_cleanup
            .compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        limiter.retain_recent();
    }
}

// ---------------------------------------------------------------------------
// Rate limiter construction
// ---------------------------------------------------------------------------

fn build_rate_limiter(
    rate: Option<f64>,
    burst: Option<u32>,
) -> PyResult<Option<Arc<IpRateLimiter>>> {
    let Some(rate) = rate else {
        if burst.is_some() {
            return Err(PyValueError::new_err(
                "rate_limit_max_burst requires rate_limit_per_ip",
            ));
        }
        return Ok(None);
    };

    if rate <= 0.0 {
        return Err(PyValueError::new_err("rate_limit_per_ip must be > 0"));
    }

    let period = Duration::try_from_secs_f64(1.0 / rate)
        .map_err(|_| PyValueError::new_err("invalid rate_limit_per_ip"))?;

    let burst = match burst {
        Some(b) => NonZeroU32::new(b)
            .ok_or_else(|| PyValueError::new_err("rate_limit_max_burst must be > 0")),
        None => Ok(NonZeroU32::MIN), // default burst = 1
    }?;

    let quota = Quota::with_period(period)
        .ok_or_else(|| PyValueError::new_err("rate_limit_per_ip is too large"))?
        .allow_burst(burst);

    Ok(Some(Arc::new(governor::RateLimiter::dashmap(quota))))
}

// ---------------------------------------------------------------------------
// TLS config
// ---------------------------------------------------------------------------

fn build_tls_config(
    certificate_chain: Vec<Vec<u8>>,
    private_key: Vec<u8>,
) -> PyResult<rustls::ServerConfig> {
    let certs: Vec<rustls::pki_types::CertificateDer<'static>> = certificate_chain
        .into_iter()
        .map(rustls::pki_types::CertificateDer::from)
        .collect();

    let key = rustls::pki_types::PrivateKeyDer::try_from(private_key)
        .map_err(|e| PyValueError::new_err(format!("invalid private key: {e}")))?;

    let provider = rustls::crypto::ring::default_provider();

    let mut tls_config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| PyValueError::new_err(format!("TLS config error: {e}")))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| PyValueError::new_err(format!("certificate error: {e}")))?;

    tls_config.alpn_protocols = vec![web_transport_quinn::ALPN.as_bytes().to_vec()];

    Ok(tls_config)
}

// ---------------------------------------------------------------------------
// SessionRequest
// ---------------------------------------------------------------------------

#[pyclass]
pub struct SessionRequest {
    inner: Option<web_transport_quinn::Request>,
    url: String,
    remote_address: (String, u16),
}

impl SessionRequest {
    pub fn new(request: web_transport_quinn::Request) -> Self {
        let url = request.url.to_string();
        let addr = request.remote_address();
        Self {
            inner: Some(request),
            url,
            remote_address: (addr.ip().to_string(), addr.port()),
        }
    }
}

#[pymethods]
impl SessionRequest {
    /// The URL requested by the client.
    #[getter]
    fn url(&self) -> String {
        self.url.clone()
    }

    /// The remote peer's ``(host, port)``.
    #[getter]
    fn remote_address(&self) -> (String, u16) {
        self.remote_address.clone()
    }

    /// Accept the session request.
    fn accept<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let request = self
            .inner
            .take()
            .ok_or_else(|| errors::SessionError::new_err("request already accepted or rejected"))?;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let session = request.ok().await.map_err(errors::map_server_error)?;
            Ok(Session::new(session))
        })
    }

    /// Reject the session request with an HTTP status code.
    #[pyo3(signature = (status_code=404))]
    fn reject<'py>(&mut self, py: Python<'py>, status_code: u16) -> PyResult<Bound<'py, PyAny>> {
        let request = self
            .inner
            .take()
            .ok_or_else(|| errors::SessionError::new_err("request already accepted or rejected"))?;
        let status = http::StatusCode::from_u16(status_code)
            .map_err(|e| PyValueError::new_err(format!("invalid status code: {e}")))?;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            // Use respond() instead of reject() so we get back a Session that
            // keeps the QUIC connection alive.  reject() drops the connection
            // immediately, which can trigger an implicit CONNECTION_CLOSE
            // before the rejection response is transmitted to the peer.
            let session = request
                .respond(status)
                .await
                .map_err(errors::map_server_error)?;
            // Close from our side. The close capsule and CONNECTION_CLOSE
            // are sent by a background task; the endpoint's wait_idle()
            // ensures they are transmitted before shutdown.
            session.close(0, b"");
            Ok(())
        })
    }
}
