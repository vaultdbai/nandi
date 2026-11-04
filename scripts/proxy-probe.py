"""Stand-in for Nandi's egress proxy, for checking that coding agents send
their traffic through it (docs/developer/sandbox-test-checklist.md).

Listens on 127.0.0.1:11435 (quit Nandi first: it holds that port), logs the
first line of every request and whether it carried proxy credentials, and
answers 403. Run with: python -I scripts/proxy-probe.py <log file> <seconds>
"""

import socket, sys, threading, time
log = open(sys.argv[1], "a", buffering=1)
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 11435)); s.listen(); s.settimeout(1)
end = time.time() + float(sys.argv[2])
def handle(c):
    c.settimeout(5)
    try:
        data = c.recv(4096).decode(errors="replace")
        lines = data.split("\r\n")
        auth = [l for l in lines if l.lower().startswith("proxy-authorization")]
        log.write(f"{lines[0]} | auth={'yes' if auth else 'no'}\n")
        c.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
    except Exception as e:
        log.write(f"error {e}\n")
    finally:
        c.close()
while time.time() < end:
    try:
        c, _ = s.accept()
        threading.Thread(target=handle, args=(c,), daemon=True).start()
    except socket.timeout:
        pass
