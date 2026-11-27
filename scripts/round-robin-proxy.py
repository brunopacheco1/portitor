# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
"""A small HTTP reverse proxy that spreads requests over several processes.

For the replica test (scenarios/two-party/run-replicas.sh) only: it plays
the load balancer in front of two processes of each component, as nginx
does in scenarios/two-party/replicas.yaml. Standard library only.

    python3 scripts/round-robin-proxy.py --log proxy.log --control 127.0.0.1:18299 \
        provider=127.0.0.1:18280=127.0.0.1:18380,127.0.0.1:18480 ...

Each listener NAME=ADDRESS=BACKEND,BACKEND,... sends each request to the
next backend in turn (round robin, per request, never sticky). A backend
that refuses the connection is skipped, and the request goes to the next
one, as nginx's `proxy_next_upstream error` does; when none answers, the
proxy answers 502. Each answer carries `x-test-backend: <backend>`.

The control address takes `POST /down/NAME[/PATH]` and `POST /up/NAME`:
while a listener is down it answers every request (or every request whose
path starts with /PATH) with 503 and a JSON error body, as a busy or
unreachable participant behind a load balancer would. The
log has one line per request: listener, method, path, backend, status.
"""

import argparse
import http.client
import http.server
import itertools
import json
import socket
import sys
import threading

# headers that belong to one connection, not to the request (RFC 9110, 7.6.1)
HOP_BY_HOP = {
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
}

LOG_LOCK = threading.Lock()
DOWN = {}  # listener name -> the path prefix that is down ("/" for all)


def log(stream, *fields):
    with LOG_LOCK:
        stream.write(" ".join(str(f) for f in fields) + "\n")
        stream.flush()


def read_body(handler):
    """The request body: by Content-Length, or chunked."""
    length = handler.headers.get("content-length")
    if length is not None:
        return handler.rfile.read(int(length))
    if "chunked" in handler.headers.get("transfer-encoding", "").lower():
        body = bytearray()
        while True:
            size = int(handler.rfile.readline().split(b";")[0].strip(), 16)
            if size == 0:
                # trailers, up to the empty line
                while handler.rfile.readline() not in (b"\r\n", b"\n", b""):
                    pass
                return bytes(body)
            body += handler.rfile.read(size)
            handler.rfile.readline()
    return None


def make_handler(name, backends, stream):
    turn = itertools.cycle(range(len(backends)))
    turn_lock = threading.Lock()

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        disable_nagle_algorithm = True

        def log_message(self, *args):  # the request log is ours, below
            pass

        def answer(self, status, headers, body, backend):
            self.send_response(status)
            for key, value in headers:
                if key.lower() not in HOP_BY_HOP and key.lower() != "content-length":
                    self.send_header(key, value)
            self.send_header("x-test-backend", backend)
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(body)
            log(stream, name, self.command, self.path, backend, status)

        def proxy(self):
            body = read_body(self)
            if not self.path.startswith("/") or self.path.startswith("//"):
                # only origin-form targets (RFC 9112, 3.2.1): the backend is
                # always one of ours, never a host named in the request
                error = json.dumps({"error": "only a path is accepted"}).encode()
                self.answer(400, [("content-type", "application/json")], error, "-")
                return
            if name in DOWN and self.path.startswith(DOWN[name]):
                error = json.dumps({"error": f"{name} is down for the test"}).encode()
                self.answer(503, [("content-type", "application/json")], error, "-")
                return
            with turn_lock:
                first = next(turn)
            headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP_BY_HOP}
            for i in range(len(backends)):
                backend = backends[(first + i) % len(backends)]
                host, port = backend.rsplit(":", 1)
                conn = http.client.HTTPConnection(host, int(port), timeout=60)
                try:
                    conn.request(self.command, self.path, body=body, headers=headers)
                except (ConnectionRefusedError, socket.timeout, OSError):
                    conn.close()
                    continue  # the next backend, as nginx does on a connection error
                try:
                    response = conn.getresponse()
                    data = response.read()
                except (OSError, http.client.HTTPException) as e:
                    conn.close()
                    error = json.dumps({"error": f"{backend}: {e}"}).encode()
                    self.answer(502, [("content-type", "application/json")], error, backend)
                    return
                conn.close()
                self.answer(response.status, response.getheaders(), data, backend)
                return
            error = json.dumps({"error": f"no backend of {name} answers"}).encode()
            self.answer(502, [("content-type", "application/json")], error, "-")

        do_GET = do_POST = do_PUT = do_DELETE = do_PATCH = do_HEAD = do_OPTIONS = proxy

    return Handler


def control_handler(names, stream):
    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *args):
            pass

        def do_POST(self):
            parts = self.path.strip("/").split("/", 2)
            status = 404
            if len(parts) >= 2 and parts[0] in ("down", "up") and parts[1] in names:
                if parts[0] == "down":
                    DOWN[parts[1]] = "/" + (parts[2] if len(parts) == 3 else "")
                else:
                    DOWN.pop(parts[1], None)
                status = 204
                log(stream, "control", *parts)
            self.send_response(status)
            self.send_header("content-length", "0")
            self.end_headers()

    return Handler


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = 1024


def serve(server):
    threading.Thread(target=server.serve_forever, daemon=True).start()


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--log", required=True)
    parser.add_argument("--control", required=True, help="host:port of the control API")
    parser.add_argument("listeners", nargs="+", help="NAME=ADDRESS=BACKEND,BACKEND,...")
    args = parser.parse_args()
    stream = open(args.log, "a", encoding="utf-8")
    names = set()
    for spec in args.listeners:
        name, address, backends = spec.split("=")
        host, port = address.rsplit(":", 1)
        names.add(name)
        serve(Server((host, int(port)), make_handler(name, backends.split(","), stream)))
    host, port = args.control.rsplit(":", 1)
    control = Server((host, int(port)), control_handler(names, stream))
    print("round-robin proxy ready", flush=True)
    try:
        control.serve_forever()
    except KeyboardInterrupt:
        sys.exit(0)


if __name__ == "__main__":
    main()
