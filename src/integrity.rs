//! Legacy RADIUS packet integrity (RFC 3579). TLS peer authentication is separate.

use std::fmt;

use openssl::{hash::MessageDigest, memcmp, pkey::PKey, sign::Signer};

use crate::radius::{Packet, HEADER_BYTES};

const ACCESS_REQUEST: u8 = 1;
const EAP_MESSAGE: u8 = 79;
const MESSAGE_AUTHENTICATOR: u8 = 80;
const AUTH_BYTES: usize = 16;
const ZERO: [u8; AUTH_BYTES] = [0; AUTH_BYTES];
pub(crate) const RADSEC_SHARED_SECRET: &[u8] = b"radsec";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    WrongPacketType,
    MalformedAttribute,
    DuplicateAuthenticator,
    MissingAuthenticator,
    InvalidAuthenticator,
    InvalidSharedSecret,
    CryptoUnavailable,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for Error {}

/// An Access-Request whose required integrity check has succeeded.
pub struct VerifiedAccessRequest<'a>(Packet<'a>);

impl<'a> VerifiedAccessRequest<'a> {
    pub fn packet(&self) -> &Packet<'a> {
        &self.0
    }
}

/// Strict UDP policy: every Access-Request must have one valid authenticator.
pub fn verify_udp_access_request<'a>(
    packet: Packet<'a>,
    shared_secret: &[u8],
) -> Result<VerifiedAccessRequest<'a>, Error> {
    verify_access(packet, shared_secret, true)
}

pub(crate) fn verify_tls_access_request(
    packet: Packet<'_>,
) -> Result<VerifiedAccessRequest<'_>, Error> {
    verify_access(packet, RADSEC_SHARED_SECRET, false)
}

fn verify_access<'a>(
    packet: Packet<'a>,
    key: &[u8],
    required: bool,
) -> Result<VerifiedAccessRequest<'a>, Error> {
    if packet.code() != ACCESS_REQUEST {
        return Err(Error::WrongPacketType);
    }
    if key.is_empty() {
        return Err(Error::InvalidSharedSecret);
    }
    let bytes = packet.wire_bytes();
    let mut offset = HEADER_BYTES;
    let mut found = None;
    let mut eap = false;
    while offset < bytes.len() {
        let header = bytes
            .get(offset..offset + 2)
            .ok_or(Error::MalformedAttribute)?;
        let length = usize::from(header[1]);
        if length < 2 || offset + length > bytes.len() {
            return Err(Error::MalformedAttribute);
        }
        match header[0] {
            MESSAGE_AUTHENTICATOR => {
                if length != AUTH_BYTES + 2 {
                    return Err(Error::MalformedAttribute);
                }
                if found.replace(offset + 2).is_some() {
                    return Err(Error::DuplicateAuthenticator);
                }
            }
            EAP_MESSAGE => eap = true,
            _ => {}
        }
        offset += length;
    }
    match found {
        Some(start) => {
            let end = start + AUTH_BYTES;
            let key = PKey::hmac(key).map_err(|_| Error::CryptoUnavailable)?;
            let mut mac =
                Signer::new(MessageDigest::md5(), &key).map_err(|_| Error::CryptoUnavailable)?;
            mac.update(&bytes[..start])
                .map_err(|_| Error::CryptoUnavailable)?;
            mac.update(&ZERO).map_err(|_| Error::CryptoUnavailable)?;
            mac.update(&bytes[end..])
                .map_err(|_| Error::CryptoUnavailable)?;
            let mut expected = ZERO;
            let length = mac
                .sign(&mut expected)
                .map_err(|_| Error::CryptoUnavailable)?;
            if length != AUTH_BYTES || !memcmp::eq(&bytes[start..end], &expected) {
                return Err(Error::InvalidAuthenticator);
            }
        }
        None if required || eap => return Err(Error::MissingAuthenticator),
        None => {}
    }
    Ok(VerifiedAccessRequest(packet))
}
