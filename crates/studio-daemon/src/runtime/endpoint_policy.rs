//! Fail-closed policy for which engine endpoints Studio will connect to.
//!
//! Built-in and local engines are reached through OS-scoped, user-local
//! transports — a Windows named pipe or a local Unix socket — which the OS
//! already authenticates by session/ownership. Studio never exposes or connects
//! the built-in engine over unauthenticated TCP.
//!
//! Remote/TCP endpoints are rejected outright: remote providers remain opt-in
//! and unsupported until their security plan, credential model, and capability
//! matrix are complete. When that lands, this is where non-loopback TLS
//! validation, server-identity checks, and rejection of embedded URL
//! credentials / plaintext endpoints belong. Until then the policy fails closed.

use susun::EngineEndpoint;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EndpointPolicyError {
    #[error(
        "remote/TCP engine endpoints are not supported yet; built-in engine access is OS-scoped (named pipe or local socket), and remote providers remain opt-in until their credential and capability model is complete"
    )]
    RemoteUnsupported,
}

/// Validates that an endpoint is one Studio is allowed to connect to. Called
/// immediately before connecting, so an unexpected TCP endpoint fails closed
/// rather than being reached.
pub fn validate_engine_endpoint(endpoint: &EngineEndpoint) -> Result<(), EndpointPolicyError> {
    match endpoint {
        EngineEndpoint::Local
        | EngineEndpoint::UnixSocket(_)
        | EngineEndpoint::WindowsNamedPipe(_) => Ok(()),
        EngineEndpoint::Tcp(_) => Err(EndpointPolicyError::RemoteUnsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_pipe_and_socket_endpoints_are_allowed() {
        assert!(validate_engine_endpoint(&EngineEndpoint::Local).is_ok());
        assert!(
            validate_engine_endpoint(&EngineEndpoint::WindowsNamedPipe(
                r"\\.\pipe\docker_engine".into()
            ))
            .is_ok()
        );
        assert!(
            validate_engine_endpoint(&EngineEndpoint::UnixSocket("/var/run/docker.sock".into()))
                .is_ok()
        );
    }

    #[test]
    fn tcp_endpoints_are_rejected_as_unsupported() {
        // Even a loopback TCP endpoint is rejected: the built-in engine is never
        // reached over TCP, and remote providers are unsupported for now.
        let Ok(tcp) = susun::TcpEndpoint::new("127.0.0.1", 2375) else {
            return;
        };
        assert_eq!(
            validate_engine_endpoint(&EngineEndpoint::Tcp(tcp)),
            Err(EndpointPolicyError::RemoteUnsupported)
        );
    }
}
