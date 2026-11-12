"""Docker exec 안에서만 사용하는 표준 라이브러리 제어 클라이언트."""

import argparse
import http.client
import json
import socket
import sys


def control(action, role=None, host=None, port=None):
    if action == "tcp":
        try:
            connection = socket.create_connection((host, port), timeout=1)
        except OSError:
            return {"connected": False}
        connection.close()
        return {"connected": True}
    if action == "dns":
        return sorted({item[4][0] for item in
                       socket.getaddrinfo(role, 8080, socket.AF_INET, socket.SOCK_STREAM)})
    if action == "snapshot":
        port, path, method = 9091, f"/{role}", "GET"
    else:
        port, path = 9090, f"/{action}"
        method = "GET" if action in ("status", "ready") else "POST"
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
    try:
        connection.request(method, path, body=b"" if method == "POST" else None,
                           headers={"Connection": "close"})
        response = connection.getresponse()
        body = response.read(65536)
        if response.status != 200:
            raise RuntimeError(f"제어 HTTP 상태 {response.status}")
        return json.loads(body)
    finally:
        connection.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("status", "ready", "release-start", "hold-completion",
                                           "release-completion", "drain", "snapshot", "dns", "tcp"))
    parser.add_argument("--role", choices=("gateway", "inference"))
    parser.add_argument("--host")
    parser.add_argument("--port", type=int)
    args = parser.parse_args()
    if args.action in ("snapshot", "dns") and args.role is None:
        parser.error("--role 필요")
    if args.action == "tcp" and (args.host is None or args.port is None):
        parser.error("--host/--port 필요")
    try:
        print(json.dumps(control(args.action, args.role, args.host, args.port)))
    except Exception as error:
        print(json.dumps({"error_type": type(error).__name__}), file=sys.stderr)
        sys.exit(1)
