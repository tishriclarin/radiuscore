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
    if let [command, path @ ..] = args {
        if command == "help" {
            return print_help(path);
        }
    }
    if let Some((flag, path)) = args.split_last() {
        if matches!(flag.as_str(), "--help" | "-h") {
            return print_help(path);
        }
    }
    match args {
        [] => return print_help(&[]),
        [flag] if matches!(flag.as_str(), "--version" | "-V") => {
            println!("radiuscore {}", env!("CARGO_PKG_VERSION"));
        }
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

fn print_help(path: &[String]) -> Result<(), String> {
    let usage = match path {
        [] => concat!(
            "RadiusCore diagnostic CLI\n\n",
            "  radiuscore ip <command>          Validate IP prefixes\n",
            "  radiuscore option82 <command>    Inspect Option 82 values\n",
            "  radiuscore radsec <command>      Verify a RadSec TLS peer\n",
            "  radiuscore help [group [command]]\n",
            "  radiuscore --version\n\n",
            "Append --help to a group or command for details.\n",
            "Operator guide: https://github.com/tishriclarin/radiuscore/blob/main/docs/operator-cli.md"
        ),
        [group] if group == "ip" => "radiuscore ip parse <network/cidr>",
        [group, command] if group == "ip" && command == "parse" => concat!(
            "radiuscore ip parse <network/cidr>\n\n",
            "Validate and print an IPv4 or IPv6 network with host bits cleared."
        ),
        [group] if group == "option82" => concat!(
            "radiuscore option82 normalize-log <0xhex-or-escaped-text>\n",
            "radiuscore option82 decode-dhcp <hex-option-payload>\n",
            "radiuscore option82 decode-radius <binary-datagram>"
        ),
        [group, command] if group == "option82" && command == "normalize-log" => concat!(
            "radiuscore option82 normalize-log <0xhex-or-escaped-text>\n\n",
            "Print log-escaped or 0x-prefixed input as lowercase hexadecimal."
        ),
        [group, command] if group == "option82" && command == "decode-dhcp" => concat!(
            "radiuscore option82 decode-dhcp <hex-option-payload>\n\n",
            "Decode the DHCP Option 82 payload after removing its outer option header."
        ),
        [group, command] if group == "option82" && command == "decode-radius" => concat!(
            "radiuscore option82 decode-radius <binary-datagram>\n\n",
            "Read one raw RADIUS datagram from a file and print recognized Option 82 IDs."
        ),
        [group] if group == "radsec" => {
            "radiuscore radsec probe <IP:port> <server-name> <ca.pem> <cert.pem> <key.pem> <sha256-pin>"
        }
        [group, command] if group == "radsec" && command == "probe" => concat!(
            "radiuscore radsec probe <IP:port> <server-name> <ca.pem> <cert.pem> <key.pem> <sha256-pin>\n\n",
            "Verify a mutual TLS connection using a 64-digit hexadecimal peer certificate pin.\n",
            "This probe sends no RADIUS request."
        ),
        _ => return Err("unknown help topic; use radiuscore --help".into()),
    };
    println!("{usage}");
    Ok(())
}
