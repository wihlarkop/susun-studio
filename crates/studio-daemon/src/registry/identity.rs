use std::net::{Ipv4Addr, Ipv6Addr};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RegistryIdentity(String);

impl RegistryIdentity {
    pub fn parse(value: &str) -> Result<Self, RegistryIdentityError> {
        validate_plain_input(value)?;
        if value.contains(['/', '\\', '?', '#', '@']) {
            return Err(RegistryIdentityError::InvalidHost);
        }

        let canonical = if let Some(rest) = value.strip_prefix('[') {
            parse_bracketed_ipv6(rest)?
        } else {
            parse_hostname_or_ipv4(value)?
        };

        Ok(Self(canonical))
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "consumed by the Phase 15e3 durable image-transfer routes"
        )
    )]
    pub fn from_image_ref(value: &str) -> Result<Self, RegistryIdentityError> {
        validate_plain_input(value)?;
        if value.contains('\\') || value.contains(['?', '#']) || value.starts_with('/') {
            return Err(RegistryIdentityError::InvalidImageReference);
        }

        let Some((first, remainder)) = value.split_once('/') else {
            return Self::parse("docker.io");
        };
        if first.is_empty() || remainder.is_empty() || first.contains('@') {
            return Err(RegistryIdentityError::InvalidImageReference);
        }

        if first == "localhost"
            || first.contains('.')
            || first.contains(':')
            || first.starts_with('[')
        {
            Self::parse(first)
        } else {
            Self::parse("docker.io")
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RegistryIdentityError {
    #[error("registry identity is empty")]
    Empty,
    #[error("registry identity contains unsupported characters")]
    UnsafeCharacters,
    #[error("registry host is invalid")]
    InvalidHost,
    #[error("registry port is invalid")]
    InvalidPort,
    #[error("image reference cannot be mapped to a registry")]
    InvalidImageReference,
}

fn validate_plain_input(value: &str) -> Result<(), RegistryIdentityError> {
    if value.is_empty() {
        return Err(RegistryIdentityError::Empty);
    }
    if value.trim() != value
        || value.chars().any(char::is_whitespace)
        || value.contains("://")
        || value.chars().any(char::is_control)
    {
        return Err(RegistryIdentityError::UnsafeCharacters);
    }
    Ok(())
}

fn parse_bracketed_ipv6(rest: &str) -> Result<String, RegistryIdentityError> {
    let (address, suffix) = rest
        .split_once(']')
        .ok_or(RegistryIdentityError::InvalidHost)?;
    let address = address
        .parse::<Ipv6Addr>()
        .map_err(|_| RegistryIdentityError::InvalidHost)?;
    if suffix.is_empty() {
        return Ok(format!("[{address}]"));
    }
    let port = suffix
        .strip_prefix(':')
        .ok_or(RegistryIdentityError::InvalidHost)
        .and_then(parse_port)?;
    Ok(format!("[{address}]:{port}"))
}

fn parse_hostname_or_ipv4(value: &str) -> Result<String, RegistryIdentityError> {
    if value.matches(':').count() > 1 {
        return Err(RegistryIdentityError::InvalidHost);
    }
    let (host, port) = match value.split_once(':') {
        Some((host, port)) => (host, Some(parse_port(port)?)),
        None => (value, None),
    };
    if host.is_empty() {
        return Err(RegistryIdentityError::InvalidHost);
    }

    let host = host.to_ascii_lowercase();
    if host.parse::<Ipv4Addr>().is_err() {
        validate_dns_name(&host)?;
    }
    let host = match host.as_str() {
        "index.docker.io" | "registry-1.docker.io" if port.is_none() => "docker.io".to_owned(),
        _ => host,
    };
    Ok(match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

fn parse_port(value: &str) -> Result<u16, RegistryIdentityError> {
    let port = value
        .parse::<u16>()
        .map_err(|_| RegistryIdentityError::InvalidPort)?;
    if port == 0 {
        return Err(RegistryIdentityError::InvalidPort);
    }
    Ok(port)
}

fn validate_dns_name(host: &str) -> Result<(), RegistryIdentityError> {
    if host.len() > 253 {
        return Err(RegistryIdentityError::InvalidHost);
    }
    for label in host.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(RegistryIdentityError::InvalidHost);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_registry_hosts_and_docker_hub_aliases() -> Result<(), RegistryIdentityError> {
        let cases = [
            ("REGISTRY.EXAMPLE", "registry.example"),
            ("registry.example:5000", "registry.example:5000"),
            ("localhost:5000", "localhost:5000"),
            ("docker.io", "docker.io"),
            ("index.docker.io", "docker.io"),
            ("registry-1.docker.io", "docker.io"),
            ("[2001:db8::1]", "[2001:db8::1]"),
            ("[2001:0DB8::1]:5000", "[2001:db8::1]:5000"),
        ];

        for (input, expected) in cases {
            let identity = RegistryIdentity::parse(input)?;
            assert_eq!(identity.as_str(), expected, "input: {input}");
        }
        Ok(())
    }

    #[test]
    fn derives_registry_identity_from_image_references() -> Result<(), RegistryIdentityError> {
        let cases = [
            ("alpine:3.20", "docker.io"),
            ("library/alpine:3.20", "docker.io"),
            ("team/app@sha256:0123", "docker.io"),
            ("registry.example/team/app:latest", "registry.example"),
            ("localhost:5000/team/app:latest", "localhost:5000"),
            ("[2001:db8::1]:5000/team/app:latest", "[2001:db8::1]:5000"),
        ];

        for (image, expected) in cases {
            let identity = RegistryIdentity::from_image_ref(image)?;
            assert_eq!(identity.as_str(), expected, "image: {image}");
        }
        Ok(())
    }

    #[test]
    fn rejects_ambiguous_or_unsafe_registry_identities() {
        let rejected = [
            "",
            " registry.example",
            "registry.example ",
            "registry example",
            "https://registry.example",
            "registry.example/team",
            "registry.example?query",
            "registry.example#fragment",
            "user@registry.example",
            "registry.example:0",
            "registry.example:70000",
            "2001:db8::1",
            "[2001:db8::1",
            "bad_label.example",
            "-bad.example",
            "bad-.example",
            ".example",
        ];

        for input in rejected {
            assert!(
                RegistryIdentity::parse(input).is_err(),
                "identity should be rejected: {input}"
            );
        }
    }

    #[test]
    fn rejects_unsafe_image_references_before_registry_derivation() {
        let rejected = [
            "",
            " https://registry.example/team/app",
            "https://registry.example/team/app",
            "user@registry.example/team/app",
            "registry.example/team/app?token=value",
            "registry.example/team/app#fragment",
            "/team/app",
        ];

        for input in rejected {
            assert!(
                RegistryIdentity::from_image_ref(input).is_err(),
                "image reference should be rejected: {input}"
            );
        }
    }
}
