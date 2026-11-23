"""Compare RadiusCore and FreeRADIUS on a local synthetic request."""
from pathlib import Path
import os
import socket
import tempfile
import subprocess
import time

CONFIG = """
localstatedir = @LAB@
run_dir = ${localstatedir}
logdir = ${localstatedir}
log {
 destination = stdout
}
client fixture {
 ipaddr = 127.0.0.1
 secret = radiuscore-isolated-fixture-only
 require_message_authenticator = yes
}
server option82_fixture {
 listen {
  type = auth
  ipaddr = 127.0.0.1
  port = @PORT@
 }
 authorize {
  if (&ADSL-Agent-Circuit-Id == 0x500000020005 && &ADSL-Agent-Remote-Id == 0x4d595f41505f31) {
   update control {
    Auth-Type := Accept
   }
  }
 }
 post-auth {
  update reply {
   Framed-IP-Address := 192.0.2.7
   Framed-Route := "198.51.100.32/29 0.0.0.0 1"
   Delegated-IPv6-Prefix := 2001:db8:100::/56
  }
 }
}
"""

work = Path(__file__).resolve().parents[1]
binary = Path(os.environ.get("RADIUSCORE_BIN", work / "target/release/radiuscore"))
dictdir = Path(os.environ.get("RADIUS_DICTIONARY_DIR", "/usr/share/freeradius"))
server_bin = os.environ.get("FREERADIUS_BIN", "freeradius")
client_bin = os.environ.get("RADCLIENT_BIN", "radclient")

with tempfile.TemporaryDirectory(prefix="radiuscore-freeradius-") as workdir:
    lab = Path(workdir)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as port_probe:
        port_probe.bind(("127.0.0.1", 0))
        server_port = port_probe.getsockname()[1]
    config = CONFIG.replace("@LAB@", str(lab)).replace("@PORT@", str(server_port))
    (lab / "radiusd.conf").write_text(config)
    with (lab / "run.log").open("w") as log, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as relay:
        relay.bind(("127.0.0.1", 0))
        relay.settimeout(5)
        server = subprocess.Popen(
            [server_bin, "-X", "-d", lab, "-D", dictdir],
            stdout=log, stderr=subprocess.STDOUT,
        )
        try:
            deadline = time.monotonic() + 5
            while "Ready to process requests" not in (lab / "run.log").read_text():
                if server.poll() is not None or time.monotonic() >= deadline:
                    raise AssertionError((lab / "run.log").read_text())
                time.sleep(0.05)
            request = lab / "request.txt"
            request.write_text(
                "User-Name = radiuscore-test\n"
                "ADSL-Agent-Circuit-Id = 0x500000020005\n"
                "ADSL-Agent-Remote-Id = 0x4d595f41505f31\n"
                "Message-Authenticator = 0x00000000000000000000000000000000\n"
            )
            expected = lab / "reply.txt"
            expected.write_text(
                "Message-Authenticator =* 0x00\n"
                "Framed-IP-Address == 192.0.2.7\n"
                'Framed-Route == "198.51.100.32/29 0.0.0.0 1"\n'
                "Delegated-IPv6-Prefix == 2001:db8:100::/56\n"
            )
            client = subprocess.Popen(
                [client_bin, "-x", "-b", "-r", "1", "-t", "4", "-D", dictdir,
                 "-d", lab, "-f", f"{request}:{expected}", f"127.0.0.1:{relay.getsockname()[1]}",
                 "auth", "radiuscore-isolated-fixture-only"],
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
            )
            try:
                packet, peer = relay.recvfrom(4096)
                received = lab / "request.bin"
                received.write_bytes(packet)
                relay.sendto(packet, ("127.0.0.1", server_port))
                response, _ = relay.recvfrom(4096)
                relay.sendto(response, peer)
                output, _ = client.communicate(timeout=8)
                assert client.returncode == 0 and "Access-Accept" in output, output
                radiuscore = subprocess.check_output(
                    [binary, "option82", "decode-radius", received],
                    text=True, timeout=5,
                )
                assert "Agent-Circuit-Id=0x500000020005" in radiuscore
                assert "Agent-Remote-Id=0x4d595f41505f31" in radiuscore
                print("FreeRADIUS 3.2.7 sent Access-Accept with the expected IPv4, routed IPv4 block, and IPv6 delegation.")
                print("RadiusCore decoded the same captured Option 82 values:")
                print(radiuscore.strip())
            finally:
                if client.poll() is None:
                    client.kill()
                    client.wait()
        finally:
            server.terminate()
            server.wait(timeout=5)
