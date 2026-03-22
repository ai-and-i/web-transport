"""Unit tests for constructor and configuration validation."""

import pytest

import web_transport


def test_server_invalid_bind_address(self_signed_cert):
    """Server(bind="not-an-address") -> ValueError."""
    cert, key = self_signed_cert
    with pytest.raises(ValueError):
        web_transport.Server(
            certificate_chain=[cert],
            private_key=key,
            bind="not-an-address",
        )


def test_server_invalid_private_key(self_signed_cert):
    """Server(private_key=b"garbage") -> ValueError."""
    cert, _ = self_signed_cert
    with pytest.raises(ValueError):
        web_transport.Server(
            certificate_chain=[cert],
            private_key=b"garbage",
            bind="[::1]:0",
        )


def test_server_invalid_certificate(self_signed_cert):
    """Server(certificate_chain=[b"garbage"]) -> ValueError."""
    _, key = self_signed_cert
    with pytest.raises(ValueError):
        web_transport.Server(
            certificate_chain=[b"garbage"],
            private_key=key,
            bind="[::1]:0",
        )


def test_client_invalid_congestion_control():
    """Client(congestion_control="invalid") -> ValueError."""
    with pytest.raises(ValueError):
        web_transport.Client(congestion_control="invalid")  # type: ignore[invalid-argument-type]


def test_server_invalid_congestion_control(self_signed_cert):
    """Server(congestion_control="invalid") -> ValueError."""
    cert, key = self_signed_cert
    with pytest.raises(ValueError):
        web_transport.Server(
            certificate_chain=[cert],
            private_key=key,
            congestion_control="invalid",  # type: ignore[invalid-argument-type]
        )


def test_invalid_idle_timeout():
    """max_idle_timeout=-1.0 -> ValueError."""
    with pytest.raises(ValueError):
        web_transport.Client(max_idle_timeout=-1.0)


def test_invalid_keep_alive_interval():
    """keep_alive_interval=-1.0 -> ValueError."""
    with pytest.raises(ValueError):
        web_transport.Client(keep_alive_interval=-1.0)


def test_client_close_code_max_valid():
    """client.close(code=2**62 - 1) is accepted (max QUIC VarInt)."""
    client = web_transport.Client()
    client.close(code=2**62 - 1)


def test_client_close_code_too_large():
    """client.close(code=2**62) -> ValueError."""
    client = web_transport.Client()
    with pytest.raises(ValueError):
        client.close(code=2**62)


def test_server_close_code_too_large(self_signed_cert):
    """server.close(code=2**62) -> ValueError."""
    cert, key = self_signed_cert
    server = web_transport.Server(
        certificate_chain=[cert],
        private_key=key,
        bind="[::1]:0",
    )
    with pytest.raises(ValueError):
        server.close(code=2**62)


def test_client_default_system_roots():
    """Client() with no cert args uses system root CAs."""
    client = web_transport.Client()
    assert client is not None


# ---------------------------------------------------------------------------
# Rate limiting parameter validation
# ---------------------------------------------------------------------------


def test_rate_limit_per_ip_zero(self_signed_cert):
    """rate_limit_per_ip=0 -> ValueError."""
    cert, key = self_signed_cert
    with pytest.raises(ValueError):
        web_transport.Server(
            certificate_chain=[cert],
            private_key=key,
            bind="[::1]:0",
            rate_limit_per_ip=0,
        )


def test_rate_limit_per_ip_negative(self_signed_cert):
    """rate_limit_per_ip=-1.0 -> ValueError."""
    cert, key = self_signed_cert
    with pytest.raises(ValueError):
        web_transport.Server(
            certificate_chain=[cert],
            private_key=key,
            bind="[::1]:0",
            rate_limit_per_ip=-1.0,
        )


def test_rate_limit_max_burst_without_rate_limiting(self_signed_cert):
    """rate_limit_max_burst without rate_limit_per_ip -> ValueError."""
    cert, key = self_signed_cert
    with pytest.raises(ValueError, match="rate_limit_max_burst"):
        web_transport.Server(
            certificate_chain=[cert],
            private_key=key,
            bind="[::1]:0",
            rate_limit_max_burst=5,
        )


def test_rate_limit_max_burst_zero(self_signed_cert):
    """rate_limit_max_burst=0 -> ValueError."""
    cert, key = self_signed_cert
    with pytest.raises(ValueError):
        web_transport.Server(
            certificate_chain=[cert],
            private_key=key,
            bind="[::1]:0",
            rate_limit_per_ip=10.0,
            rate_limit_max_burst=0,
        )


def test_rate_limit_valid_config(self_signed_cert):
    """Valid rate limit config succeeds (burst defaults to 1)."""
    cert, key = self_signed_cert
    server = web_transport.Server(
        certificate_chain=[cert],
        private_key=key,
        bind="[::1]:0",
        rate_limit_per_ip=10.0,
    )
    assert server is not None


def test_rate_limit_valid_config_with_burst(self_signed_cert):
    """Valid rate limit config with explicit burst succeeds."""
    cert, key = self_signed_cert
    server = web_transport.Server(
        certificate_chain=[cert],
        private_key=key,
        bind="[::1]:0",
        rate_limit_per_ip=10.0,
        rate_limit_max_burst=3,
    )
    assert server is not None
