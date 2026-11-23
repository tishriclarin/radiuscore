use std::{
    error::Error,
    fs,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use openssl::{
    ssl::{SslConnector, SslMethod, SslVerifyMode},
    x509::X509,
};
use radiuscore::{
    integrity::{self, verify_udp_access_request},
    option82::decode_hex,
    radius::Packet,
    radsec::{certificate_pin, Client, Identity, Server},
};

const DEADLINE: Duration = Duration::from_secs(4);
const UDP_SECRET: &[u8] = b"isolated-fixture-radius-secret";
const ACCESS: [u8; 20] = [
    1, 7, 0, 20, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];
type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

#[test]
fn requires_a_single_valid_udp_message_authenticator() {
    let data = decode_hex(
        "0137002b000102030405060708090a0b0c0d0e0f0105626f6250121291c22f5b2039dd10eb3199558bea39",
    )
    .unwrap();
    let packet = Packet::decode(&data).unwrap();
    assert!(verify_udp_access_request(packet, UDP_SECRET).is_ok());
    let packet = Packet::decode(&ACCESS).unwrap();
    assert_eq!(
        verify_udp_access_request(packet, UDP_SECRET).err(),
        Some(integrity::Error::MissingAuthenticator)
    );
    let mut changed = data.clone();
    changed[23] ^= 1;
    assert_eq!(
        verify_udp_access_request(Packet::decode(&changed).unwrap(), UDP_SECRET).err(),
        Some(integrity::Error::InvalidAuthenticator)
    );
    let mut duplicate = data.clone();
    duplicate.extend_from_slice(&data[25..]);
    duplicate[3] = duplicate.len() as u8;
    assert_eq!(
        verify_udp_access_request(Packet::decode(&duplicate).unwrap(), UDP_SECRET).err(),
        Some(integrity::Error::DuplicateAuthenticator)
    );
}

struct Pki(PathBuf);

impl Drop for Pki {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Pki {
    fn fixture() -> Result<Self, Box<dyn Error + Send + Sync>> {
        let mut suffix = [0u8; 8];
        openssl::rand::rand_bytes(&mut suffix)?;
        let dir = std::env::temp_dir().join(format!(
            "radiuscore-radsec-{:016x}",
            u64::from_be_bytes(suffix)
        ));
        fs::create_dir(&dir)?;
        let pki = Self(dir);
        pki.openssl(&[
            "req",
            "-x509",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:prime256v1",
            "-nodes",
            "-days",
            "2",
            "-subj",
            "/CN=RadiusCore lab root",
            "-addext",
            "basicConstraints=critical,CA:TRUE",
            "-addext",
            "keyUsage=critical,keyCertSign,cRLSign",
            "-out",
            "ca.pem",
            "-keyout",
            "ca.key",
        ])?;
        for (name, usage, serial) in [
            ("server", "serverAuth", "101"),
            ("client", "clientAuth", "102"),
        ] {
            let subject = format!("/CN={name}.radiuscore.test");
            let key = format!("{name}.key");
            let csr = format!("{name}.csr");
            let cert = format!("{name}.pem");
            let ext = format!("{name}.ext");
            fs::write(pki.0.join(&ext), format!("basicConstraints=CA:FALSE\nkeyUsage=digitalSignature\nextendedKeyUsage={usage}\nsubjectAltName=DNS:{name}.radiuscore.test\n"))?;
            pki.openssl(&[
                "req",
                "-new",
                "-newkey",
                "ec",
                "-pkeyopt",
                "ec_paramgen_curve:prime256v1",
                "-nodes",
                "-subj",
                &subject,
                "-out",
                &csr,
                "-keyout",
                &key,
            ])?;
            pki.openssl(&[
                "x509",
                "-req",
                "-in",
                &csr,
                "-CA",
                "ca.pem",
                "-CAkey",
                "ca.key",
                "-set_serial",
                serial,
                "-days",
                "2",
                "-extfile",
                &ext,
                "-out",
                &cert,
            ])?;
        }
        Ok(pki)
    }

    fn openssl(&self, arguments: &[&str]) -> TestResult {
        let output = Command::new("openssl")
            .current_dir(&self.0)
            .args(arguments)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned().into())
        }
    }

    fn read(&self, file: impl AsRef<Path>) -> std::io::Result<Vec<u8>> {
        fs::read(self.0.join(file))
    }

    fn identity(&self, name: &str) -> Result<Identity, Box<dyn Error + Send + Sync>> {
        Identity::from_pem(
            &self.read(format!("{name}.pem"))?,
            &self.read(format!("{name}.key"))?,
        )
    }

    fn pin(&self, name: &str) -> Result<[u8; 32], Box<dyn Error + Send + Sync>> {
        certificate_pin(&X509::from_pem(&self.read(format!("{name}.pem"))?)?)
    }
}

#[test]
fn radsec_verifies_certificates_and_reads_multiple_bounded_frames() -> TestResult {
    let pki = Pki::fixture()?;
    let roots = pki.read("ca.pem")?;
    let client_pin = pki.pin("client")?;
    let server = Server::new(pki.identity("server")?, &roots, vec![client_pin])?;
    let client = Client::new(pki.identity("client")?, &roots, vec![pki.pin("server")?])?;
    let listen = TcpListener::bind(("127.0.0.1", 0))?;
    let address = listen.local_addr()?;
    let worker = thread::spawn(move || -> TestResult {
        let mut link = server.accept(listen.accept()?.0, DEADLINE)?;
        assert_eq!(link.peer_pin(), client_pin);
        let mut scratch = [0u8; 4096];
        for _ in 0..2 {
            let request = link.read_access_request(&mut scratch, DEADLINE)?;
            assert_eq!(request.packet().wire_bytes(), ACCESS);
        }
        Ok(())
    });
    let mut link = client.connect(
        TcpStream::connect(address)?,
        "server.radiuscore.test",
        DEADLINE,
    )?;
    link.write_frame(&ACCESS, DEADLINE)?;
    link.write_frame(&ACCESS, DEADLINE)?;
    worker.join().unwrap()
}

#[test]
fn radsec_rejects_wrong_hostname_certificate_pin_and_missing_client_certificate() -> TestResult {
    let pki = Pki::fixture()?;
    let roots = pki.read("ca.pem")?;
    let server_pin = pki.pin("server")?;
    for (hostname, pin) in [
        ("wrong.radiuscore.test", server_pin),
        ("server.radiuscore.test", [0; 32]),
    ] {
        let server = Server::new(pki.identity("server")?, &roots, vec![pki.pin("client")?])?;
        let client = Client::new(pki.identity("client")?, &roots, vec![pin])?;
        let listen = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listen.local_addr()?;
        let worker = thread::spawn(move || server.accept(listen.accept().unwrap().0, DEADLINE));
        assert!(client
            .connect(TcpStream::connect(address)?, hostname, DEADLINE)
            .is_err());
        let _ = worker.join().unwrap();
    }
    let server = Server::new(pki.identity("server")?, &roots, vec![pki.pin("client")?])?;
    let mut bare = SslConnector::builder(SslMethod::tls_client())?;
    bare.cert_store_mut().add_cert(X509::from_pem(&roots)?)?;
    bare.set_verify(SslVerifyMode::PEER);
    let listen = TcpListener::bind(("127.0.0.1", 0))?;
    let address = listen.local_addr()?;
    let worker = thread::spawn(move || server.accept(listen.accept().unwrap().0, DEADLINE));
    let _ = bare
        .build()
        .connect("server.radiuscore.test", TcpStream::connect(address)?);
    assert!(worker.join().unwrap().is_err());
    Ok(())
}

#[test]
fn cli_probe_connects_to_an_authorized_radsec_peer() -> TestResult {
    let pki = Pki::fixture()?;
    let roots = pki.read("ca.pem")?;
    let server = Server::new(pki.identity("server")?, &roots, vec![pki.pin("client")?])?;
    let listen = TcpListener::bind(("127.0.0.1", 0))?;
    let address = listen.local_addr()?;
    let worker = thread::spawn(move || server.accept(listen.accept().unwrap().0, DEADLINE));
    let output = Command::new(env!("CARGO_BIN_EXE_radiuscore"))
        .args([
            "radsec",
            "probe",
            &address.to_string(),
            "server.radiuscore.test",
        ])
        .arg(pki.0.join("ca.pem"))
        .arg(pki.0.join("client.pem"))
        .arg(pki.0.join("client.key"))
        .arg(radiuscore::option82::hex(&pki.pin("server")?))
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("RadSec TLS peer verified"));
    assert!(worker.join().unwrap().is_ok());
    Ok(())
}
