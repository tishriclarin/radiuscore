use radiuscore::{
    ipam::Prefix,
    option82::{decode_dhcp_option, decode_escaped_log, decode_hex, hex},
    radius::{Packet, CIRCUIT_SLOT, REMOTE_SLOT},
    radsec::{self, CertificatePin},
    rewrite::RewritePlan,
};
use std::{
    env,
    fs::{self, File},
    io::Read,
    net::{SocketAddr, TcpStream},
    process::ExitCode,
    time::Duration,
};

fn main() -> ExitCode {
    let args: Vec<_> = env::args().skip(1).collect();
    if let Err(error) = run(&args) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run(args: &[String]) -> Result<(), String> {
    match args {
        [] => print_help(),
        [flag] if flag == "--help" => print_help(),
        [flag] if flag == "--version" => println!("radiuscore {}", env!("CARGO_PKG_VERSION")),
        [group, cmd, value] if group == "ip" && cmd == "parse" => {
            let prefix = value
                .parse::<Prefix>()
                .map_err(|error| format!("invalid prefix: {error:?}"))?;
            println!("{prefix}");
        }
        [group, cmd, value] if group == "option82" && cmd == "normalize-log" => {
            let raw = if value.starts_with("0x") || value.starts_with("0X") {
                decode_hex(value)
            } else {
                decode_escaped_log(value)
            }
            .map_err(|error| format!("invalid diagnostic value: {error:?}"))?;
            println!("0x{}", hex(&raw));
        }
        [group, cmd, value] if group == "option82" && cmd == "decode-dhcp" => {
            let raw = decode_hex(value).map_err(|error| format!("invalid hex: {error:?}"))?;
            let ids = decode_dhcp_option(&raw)
                .map_err(|error| format!("invalid Option 82: {error:?}"))?;
            if let Some(circuit_id) = ids.circuit_id {
                println!("Agent-Circuit-Id=0x{}", hex(circuit_id));
            }
            if let Some(remote_id) = ids.remote_id {
                println!("Agent-Remote-Id=0x{}", hex(remote_id));
            }
        }
        [group, cmd, value] if group == "option82" && cmd == "decode-radius" => {
            let mut bytes = Vec::new();
            File::open(value)
                .and_then(|file| file.take(4097).read_to_end(&mut bytes))
                .map_err(|error| format!("cannot read diagnostic packet: {error}"))?;
            if bytes.len() > 4096 {
                return Err("diagnostic datagram exceeds 4096 bytes".into());
            }
            let packet = Packet::decode(&bytes)
                .map_err(|error| format!("invalid RADIUS frame: {error:?}"))?;
            let ids = packet
                .option82_slots()
                .map_err(|error| format!("invalid attributes: {error:?}"))?;
            RewritePlan::compile(&[])
                .expect("an empty diagnostic plan is valid")
                .worker()
                .then_lookup(&ids, |working| {
                    for (slot, name) in [
                        (CIRCUIT_SLOT, "Agent-Circuit-Id"),
                        (REMOTE_SLOT, "Agent-Remote-Id"),
                    ] {
                        if let Some(bytes) = working.attribute(slot)? {
                            println!("{name}=0x{}", hex(bytes));
                        }
                    }
                    Ok::<(), radiuscore::rewrite::Error>(())
                })
                .and_then(|result| result)
                .map_err(|error| format!("invalid diagnostic packet: {error:?}"))?;
        }
        [group, cmd, address, hostname, ca, cert, key, pin]
            if group == "radsec" && cmd == "probe" =>
        {
            probe_radsec(address, hostname, ca, cert, key, pin)?;
        }
        _ => return Err("unknown arguments; use radiuscore --help".into()),
    }
    Ok(())
}

fn probe_radsec(
    address: &str,
    hostname: &str,
    ca: &str,
    cert: &str,
    key: &str,
    pin: &str,
) -> Result<(), String> {
    let address: SocketAddr = address.parse().map_err(|_| "invalid socket address")?;
    let pin: CertificatePin = decode_hex(pin)
        .map_err(|_| "expected a hexadecimal certificate SHA-256 pin")?
        .try_into()
        .map_err(|_| "expected a 32-byte certificate SHA-256 pin")?;
    let identity = radsec::Identity::from_pem(
        &fs::read(cert).map_err(|error| format!("cannot read client certificate: {error}"))?,
        &fs::read(key).map_err(|error| format!("cannot read client key: {error}"))?,
    )
    .map_err(|_| "cannot load client certificate and private key")?;
    let roots = fs::read(ca).map_err(|error| format!("cannot read CA bundle: {error}"))?;
    let client =
        radsec::Client::new(identity, &roots, vec![pin]).map_err(|error| error.to_string())?;
    let timeout = Duration::from_secs(5);
    let stream =
        TcpStream::connect_timeout(&address, timeout).map_err(|error| error.to_string())?;
    let connection = client
        .connect(stream, hostname, timeout)
        .map_err(|error| error.to_string())?;
    println!("RadSec TLS peer verified for {hostname}");
    println!("Peer certificate SHA-256: {}", hex(&connection.peer_pin()));
    Ok(())
}

fn print_help() {
    println!("radiuscore ip parse <network/cidr>");
    println!("radiuscore radsec probe <IP:port> <server-name> <ca.pem> <cert.pem> <key.pem> <sha256-pin>");
    println!("radiuscore option82 normalize-log <0xhex-or-escaped-text>");
    println!("radiuscore option82 decode-dhcp <hex-option-payload>");
    println!("radiuscore option82 decode-radius <binary-datagram>");
}
