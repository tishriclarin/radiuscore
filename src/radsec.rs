//! Mutually authenticated RADIUS over TLS transport (RFC 6614).
//!
//! Peer certificates must pass both the configured CA verification and the
//! SHA-256 certificate allowlist. There is no cleartext fallback.

use std::{
    io::{self, Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

use openssl::{
    hash::MessageDigest,
    memcmp,
    pkey::{PKey, Private},
    ssl::{
        SslAcceptor, SslConnector, SslMethod, SslOptions, SslSessionCacheMode, SslStream,
        SslVerifyMode, SslVersion,
    },
    x509::{verify::X509CheckFlags, X509},
};

use crate::{
    integrity::{self, VerifiedAccessRequest},
    radius::{Packet, HEADER_BYTES, MAX_PACKET_BYTES},
};

pub const DEFAULT_PORT: u16 = 2083;
pub type CertificatePin = [u8; 32];
type BoxError = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, BoxError>;

pub struct Identity {
    chain: Vec<X509>,
    key: PKey<Private>,
}

impl Identity {
    pub fn from_pem(chain: &[u8], key: &[u8]) -> Result<Self> {
        let chain = X509::stack_from_pem(chain)?;
        if chain.is_empty() {
            return Err(invalid_input("certificate chain cannot be empty").into());
        }
        Ok(Self {
            chain,
            key: PKey::private_key_from_pem(key)?,
        })
    }
}

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub fn certificate_pin(certificate: &X509) -> Result<CertificatePin> {
    certificate
        .digest(MessageDigest::sha256())?
        .as_ref()
        .try_into()
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid certificate fingerprint length",
            )
            .into()
        })
}

fn configure(
    context: &mut openssl::ssl::SslContextBuilder,
    identity: Identity,
    trusted_roots: &[u8],
    pins: &[CertificatePin],
) -> Result<()> {
    let roots = X509::stack_from_pem(trusted_roots)?;
    if roots.is_empty() || pins.is_empty() {
        return Err(invalid_input("RadSec requires CA roots and at least one peer pin").into());
    }
    context.set_min_proto_version(Some(SslVersion::TLS1_2))?;
    context.set_options(
        SslOptions::NO_RENEGOTIATION | SslOptions::NO_TICKET | SslOptions::NO_COMPRESSION,
    );
    context.set_session_cache_mode(SslSessionCacheMode::OFF);
    context.set_certificate(&identity.chain[0])?;
    context.set_private_key(&identity.key)?;
    context.check_private_key()?;
    for cert in identity.chain.into_iter().skip(1) {
        context.add_extra_chain_cert(cert)?;
    }
    // Replace the connector's ambient system trust roots with the explicit bundle.
    let mut store = openssl::x509::store::X509StoreBuilder::new()?;
    for root in roots {
        store.add_cert(root)?;
    }
    context.set_cert_store(store.build());
    Ok(())
}

pub struct Server {
    tls: SslAcceptor,
    client_pins: Vec<CertificatePin>,
}

impl Server {
    pub fn new(
        identity: Identity,
        trusted_roots: &[u8],
        client_pins: Vec<CertificatePin>,
    ) -> Result<Self> {
        let mut tls = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls_server())?;
        configure(&mut tls, identity, trusted_roots, &client_pins)?;
        tls.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
        Ok(Self {
            tls: tls.build(),
            client_pins,
        })
    }

    pub fn accept(&self, stream: TcpStream, timeout: Duration) -> Result<Connection> {
        let stream = DeadlineStream::new(stream, timeout)?;
        let tls = self
            .tls
            .accept(stream)
            .map_err(|error| io::Error::other(error.to_string()))?;
        Connection::new(tls, &self.client_pins)
    }
}

pub struct Client {
    tls: SslConnector,
    server_pins: Vec<CertificatePin>,
}

impl Client {
    pub fn new(
        identity: Identity,
        trusted_roots: &[u8],
        server_pins: Vec<CertificatePin>,
    ) -> Result<Self> {
        let mut tls = SslConnector::builder(SslMethod::tls_client())?;
        configure(&mut tls, identity, trusted_roots, &server_pins)?;
        tls.set_verify(SslVerifyMode::PEER);
        Ok(Self {
            tls: tls.build(),
            server_pins,
        })
    }

    pub fn connect(
        &self,
        stream: TcpStream,
        server_name: &str,
        timeout: Duration,
    ) -> Result<Connection> {
        if server_name.is_empty() {
            return Err(invalid_input("RadSec requires an expected server name").into());
        }
        let stream = DeadlineStream::new(stream, timeout)?;
        let mut settings = self.tls.configure()?.into_ssl(server_name)?;
        settings.param_mut().set_hostflags(
            X509CheckFlags::NEVER_CHECK_SUBJECT | X509CheckFlags::NO_PARTIAL_WILDCARDS,
        );
        let tls = settings
            .connect(stream)
            .map_err(|error| io::Error::other(error.to_string()))?;
        Connection::new(tls, &self.server_pins)
    }
}

pub struct Connection {
    tls: SslStream<DeadlineStream>,
    peer_pin: CertificatePin,
    failed: bool,
}

impl Connection {
    fn new(tls: SslStream<DeadlineStream>, allowed: &[CertificatePin]) -> Result<Self> {
        let peer = tls.ssl().peer_certificate().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "RadSec peer certificate required",
            )
        })?;
        let peer_pin = certificate_pin(&peer)?;
        if !allowed.iter().any(|pin| memcmp::eq(pin, &peer_pin)) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "RadSec peer certificate is not authorized",
            )
            .into());
        }
        Ok(Self {
            tls,
            peer_pin,
            failed: false,
        })
    }

    /// Use the pin to select the configured peer policy.
    pub fn peer_pin(&self) -> CertificatePin {
        self.peer_pin
    }

    pub fn read_frame<'a>(
        &mut self,
        output: &'a mut [u8],
        timeout: Duration,
    ) -> io::Result<Packet<'a>> {
        if self.failed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "RadSec stream is unusable",
            ));
        }
        if output.len() < MAX_PACKET_BYTES {
            return Err(invalid_input("RadSec read buffer must hold 4096 bytes"));
        }
        self.tls.get_mut().reset(timeout)?;
        // On any partial or malformed frame, the TCP stream must be discarded.
        self.failed = true;
        self.tls.read_exact(&mut output[..4])?;
        let size = usize::from(u16::from_be_bytes([output[2], output[3]]));
        if !(HEADER_BYTES..=MAX_PACKET_BYTES).contains(&size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid RadSec packet length",
            ));
        }
        self.tls.read_exact(&mut output[4..size])?;
        let packet = Packet::decode(&output[..size])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid RadSec frame"))?;
        self.failed = false;
        Ok(packet)
    }

    pub fn read_access_request<'a>(
        &mut self,
        output: &'a mut [u8],
        timeout: Duration,
    ) -> Result<VerifiedAccessRequest<'a>> {
        Ok(integrity::verify_tls_access_request(
            self.read_frame(output, timeout)?,
        )?)
    }

    pub fn write_frame(&mut self, packet: &[u8], timeout: Duration) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "RadSec stream is unusable",
            ));
        }
        let framed = Packet::decode(packet).map_err(|_| invalid_input("invalid RadSec frame"))?;
        if framed.wire_bytes().len() != packet.len() {
            return Err(invalid_input(
                "RadSec frames cannot include trailing padding",
            ));
        }
        self.tls.get_mut().reset(timeout)?;
        self.failed = true;
        self.tls.write_all(packet)?;
        self.tls.flush()?;
        self.failed = false;
        Ok(())
    }
}

#[derive(Debug)]
struct DeadlineStream {
    stream: TcpStream,
    deadline: Instant,
}

impl DeadlineStream {
    fn new(stream: TcpStream, timeout: Duration) -> io::Result<Self> {
        let mut this = Self {
            stream,
            deadline: Instant::now(),
        };
        this.reset(timeout)?;
        Ok(this)
    }

    fn reset(&mut self, timeout: Duration) -> io::Result<()> {
        if timeout.is_zero() {
            return Err(invalid_input("RadSec timeout must be positive"));
        }
        self.deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| invalid_input("invalid RadSec timeout"))?;
        Ok(())
    }

    fn time_left(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|time| !time.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "RadSec operation deadline passed")
            })
    }
}

impl Read for DeadlineStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.time_left()?))?;
        self.stream.read(buffer)
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.time_left()?))?;
        self.stream.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}
