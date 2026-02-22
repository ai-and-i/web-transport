use std::time::Duration;

use anyhow::Result;

use crate::browser::{JsTestResult, TestContext};
use crate::cert::{self, TestCert};
use crate::server::{self, ServerHandler, TestServer};

/// Orchestrates certificate generation, server startup, and browser context
/// creation for a single test.
pub struct TestHarness {
    pub server: TestServer,
    pub context: TestContext,
    pub cert: TestCert,
}

/// Set up a complete test environment: generate a cert, start a server with the
/// given handler, launch (or reuse) the browser, and create an isolated context.
pub async fn setup(handler: ServerHandler) -> Result<TestHarness> {
    let cert = cert::generate();
    let server = server::start(&cert, handler).await?;
    let context = TestContext::new().await?;

    Ok(TestHarness {
        server,
        context,
        cert,
    })
}

impl TestHarness {
    /// Evaluate JavaScript test code in the browser and return the result.
    pub async fn run_js(&self, js_code: &str, timeout: Duration) -> Result<JsTestResult> {
        self.context
            .run_js_test(&self.server.url, &self.cert.fingerprint, js_code, timeout)
            .await
    }

    /// Evaluate JavaScript test code and assert that it succeeded.
    pub async fn run_js_ok(&self, js_code: &str, timeout: Duration) {
        let result = self
            .run_js(js_code, timeout)
            .await
            .unwrap_or_else(|e| panic!("JS test failed with error: {e:#}"));
        assert!(result.success, "JS test failed: {}", result.message);
    }

    /// Dispose of the browser context and shut down the server.
    pub async fn teardown(self) {
        self.context.dispose().await;
        self.server.shutdown().await;
    }
}

// Re-export handler constructors for convenience.
pub use crate::server::{echo_handler, idle_handler, immediate_close_handler};
