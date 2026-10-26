"""A fake hosted registry: an https server with a throwaway self-signed certificate.

The team binary trusts only the public web roots, so it must NOT trust this
certificate. That is the property the tests assert: a client that does not trust
the server opens a connection, fails the handshake and sends no request. The
server counts all three (`connections`, `handshake_failures`, `requests`).
It can still serve routes to a client that does trust the certificate (the
selftest does), so a future trust hook in the binary needs no new fake.
"""

from __future__ import annotations

import http.server
import json
import os
import socket
import ssl
import subprocess
import threading


def make_cert(directory: str) -> tuple[str, str]:
    """A self-signed certificate for 127.0.0.1 (needs openssl, as the launcher already does)."""
    cert, key = os.path.join(directory, "reg.crt"), os.path.join(directory, "reg.key")
    r = subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2",
                        "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1",
                        "-keyout", key, "-out", cert], capture_output=True)
    if r.returncode != 0:
        raise RuntimeError("openssl cannot make the registry certificate: " + r.stderr.decode()[-300:])
    return cert, key


class FakeRegistry:
    def __init__(self, directory: str, routes: dict | None = None) -> None:
        os.makedirs(directory, exist_ok=True)
        self.cert, self.key = make_cert(directory)
        self.routes = routes or {}      # path -> (status, body text)
        self.connections = 0            # TCP connections accepted
        self.handshake_failures = 0     # TLS handshakes that failed (client distrusts the cert)
        self.requests: list[dict] = []  # HTTP requests actually received
        self._srv: http.server.ThreadingHTTPServer | None = None
        self.port = 0

    @property
    def url(self) -> str:
        return f"https://127.0.0.1:{self.port}"

    def start(self) -> "FakeRegistry":
        owner = self
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(self.cert, self.key)

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def _serve(self):
                n = int(self.headers.get("content-length") or 0)
                body = self.rfile.read(n).decode(errors="replace") if n else ""
                owner.requests.append({"method": self.command, "path": self.path, "body": body})
                status, text = owner.routes.get(self.path, (404, "{}"))
                data = text.encode()
                self.send_response(status)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            do_GET = do_POST = _serve

        class Server(http.server.ThreadingHTTPServer):
            daemon_threads = True

            def get_request(self):
                sock, addr = self.socket.accept()
                owner.connections += 1
                return sock, addr

            def process_request_thread(self, request, client_address):
                try:
                    request.settimeout(5)
                    tls = ctx.wrap_socket(request, server_side=True)
                except (ssl.SSLError, OSError):
                    owner.handshake_failures += 1
                    try:
                        request.close()
                    except OSError:
                        pass
                    return
                super().process_request_thread(tls, client_address)

        self._srv = Server(("127.0.0.1", 0), Handler)
        self.port = self._srv.server_address[1]
        threading.Thread(target=self._srv.serve_forever, daemon=True).start()
        return self

    def stop(self) -> None:
        if self._srv:
            self._srv.shutdown()
            self._srv.server_close()
            self._srv = None

    def quiet(self) -> bool:
        """Nothing at all reached this server."""
        return self.connections == 0 and not self.requests

    def json_route(self, path: str, doc, status: int = 200) -> None:
        self.routes[path] = (status, json.dumps(doc))
