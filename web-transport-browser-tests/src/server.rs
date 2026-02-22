use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

use anyhow::Result;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use web_transport_quinn::{RecvStream, SendStream};

use crate::cert::TestCert;

/// A boxed async handler invoked for each accepted WebTransport session.
pub type ServerHandler = Box<
    dyn Fn(web_transport_quinn::Session) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + Sync
        + 'static,
>;

/// A running WebTransport test server.
pub struct TestServer {
    pub addr: SocketAddr,
    pub url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl TestServer {
    /// Shut down the server and wait for the accept loop to exit.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Start a WebTransport server on a random port using the given certificate and handler.
pub async fn start(cert: &TestCert, handler: ServerHandler) -> Result<TestServer> {
    let addr: SocketAddr = "[::1]:0".parse().unwrap();

    let server = web_transport_quinn::ServerBuilder::new()
        .with_addr(addr)
        .with_certificate(cert.chain.clone(), cert.key.clone_key())?;

    let actual_addr = server.local_addr()?;
    let url = format!("https://localhost:{}", actual_addr.port());

    tracing::debug!(%url, "test server listening");

    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

    let task = tokio::spawn(async move {
        let mut server = server;
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                request = server.accept() => {
                    let Some(request) = request else { break };
                    let session = match request.ok().await {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(?e, "failed to accept session");
                            continue;
                        }
                    };
                    let fut = handler(session);
                    tokio::spawn(fut);
                }
            }
        }
    });

    Ok(TestServer {
        addr: actual_addr,
        url,
        shutdown_tx: Some(shutdown_tx),
        task: Some(task),
    })
}

/// A handler that echoes bidirectional streams back to the client.
pub fn echo_handler() -> ServerHandler {
    Box::new(|session| {
        Box::pin(async move {
            if let Err(e) = echo_session(session).await {
                tracing::debug!(?e, "echo session ended");
            }
        })
    })
}

async fn echo_session(session: web_transport_quinn::Session) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    loop {
        tokio::select! {
            stream = session.accept_bi() => {
                let (mut send, mut recv): (SendStream, RecvStream) = stream?;
                tokio::spawn(async move {
                    match recv.read_to_end(1024 * 1024).await {
                        Ok(buf) => {
                            let _ = send.write_all(&buf).await;
                            let _ = send.shutdown().await;
                        }
                        Err(e) => {
                            tracing::debug!(?e, "echo read failed");
                        }
                    }
                });
            }
            datagram = session.read_datagram() => {
                let data = datagram?;
                session.send_datagram(data)?;
            }
        }
    }
}

/// A handler that accepts the session and immediately closes it.
pub fn immediate_close_handler(code: u32, reason: &'static str) -> ServerHandler {
    Box::new(move |session| {
        Box::pin(async move {
            session.close(code, reason.as_bytes());
        })
    })
}

/// A handler that accepts the session and holds it open until the client disconnects.
pub fn idle_handler() -> ServerHandler {
    Box::new(|session| {
        Box::pin(async move {
            let _ = session.closed().await;
        })
    })
}
