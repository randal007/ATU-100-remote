#!/usr/bin/env python3
"""A simulated ESP32 bridge, for trying atutop's settings without the board.

    ./esp32_sim.py TUNER_PORT [USB_LINK] [--watch TUNER_PID]

TUNER_PORT is the simulated tuner's pseudo terminal (host_test with PTY=1).
This makes the bridge's "USB" port, a pseudo terminal linked at USB_LINK
(default /tmp/atu100-esp32), and once "connected" to Wi-Fi a TCP bridge on
127.0.0.1:7000, which stands in for the ESP32's address on the network.

It also serves atutop's browser version (tui/dist, built with trunk) on its
web port, with a WebSocket for the tuner lines, as the real bridge does on
port 81; a browser counts as on the bridge's hotspot (allowed to change the
Wi-Fi settings) until the bridge has joined a network.

It behaves as the real bridge firmware does (README.md, "ESP32 bridge"):
lines are passed whole between the tuner and every client; lines with an
"ESP32" field are answered here and never reach the tuner; Wi-Fi settings
change over USB only; {"ESP32":"Find"} on UDP port 7001 is answered with
where to connect. A fixed IP address is kept and reported, but the bridge
stays on 127.0.0.1 here. Pretend networks: VE7NHW-shack (password hamradio),
two others with unknown passwords, and an open one.

ATU100_SIM_LAN=1: listen on the home network too, so a phone on the same
network can open the web page at http://THIS_COMPUTER:8080/ (the firewall
must let 8080 and 8081 in); the bridge then reports this computer's
network address as its own.

--watch: started by atutop, which passes the simulated tuner's process id.
When atutop goes (even without quitting properly) this stops the tuner,
removes USB_LINK and ends.
"""

import base64
import hashlib
import html
import http.server
import struct
import json
import os
import threading
import pty
import selectors
import signal
import socket
import sys
import termios
import time
import tty

VERSION = 'sim'
PORT = 7000
FIND_PORT = 7001
WEB_PORT = 8080  # the real bridge uses 80 and 81; below 1024 needs root here
LAN = os.environ.get('ATU100_SIM_LAN') == '1'
LISTEN = '0.0.0.0' if LAN else '127.0.0.1'


def lan_address():
    """This computer's address on the home network (no packet is sent)"""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(('192.0.2.1', 9))  # any outside address: picks the outgoing interface
        return s.getsockname()[0]
    except OSError:
        return '127.0.0.1'
    finally:
        s.close()


ADDRESS = lan_address() if LAN else '127.0.0.1'  # where the bridge says it is
HOST = 'atu100.local'
HOTSPOT = 'ATU100-setup'
HOTSPOT_IP = '192.168.73.1'  # pretend: nothing listens there on this PC
NETWORKS = [  # SSID, RSSI, password (None = open)
    ('VE7NHW-shack', -47, 'hamradio'),
    ('TELUS4471', -71, '?'),
    ('Neighbours-5G', -80, '?'),
    ('Library', -86, None),
]


def raw(fd):
    tty.setraw(fd)
    attrs = termios.tcgetattr(fd)
    attrs[3] &= ~termios.ECHO
    termios.tcsetattr(fd, termios.TCSANOW, attrs)


class Client:
    def __init__(self, name, fd=None, sock=None):
        self.name, self.fd, self.sock = name, fd, sock
        self.buf = b''

    def send(self, data):
        try:
            if self.sock:
                self.sock.sendall(data)
            else:
                os.write(self.fd, data)
        except OSError:
            pass


WEB_DIR = os.environ.get('ATU100_WEB_DIR', os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                       '../../../tui/dist'))


class WsClient(Client):
    """A browser on the WebSocket: RFC 6455 text frames, one or more lines each"""
    def __init__(self, sock):
        super().__init__('WebSocket', sock=sock)
        self.raw = b''
        self.open = False

    def send(self, data):
        if not self.open:
            return
        n = len(data)
        head = bytes([0x81]) + (bytes([n]) if n < 126 else
                                bytes([126]) + struct.pack('>H', n) if n < 65536 else
                                bytes([127]) + struct.pack('>Q', n))
        try:
            self.sock.sendall(head + data)
        except OSError:
            pass

    def feed(self, data):
        """Bytes from the socket; returns the text received, or None when it closed"""
        self.raw += data
        if not self.open:
            if b'\r\n\r\n' not in self.raw:
                return b''
            head, self.raw = self.raw.split(b'\r\n\r\n', 1)
            key = next((l.split(b':', 1)[1].strip() for l in head.split(b'\r\n')
                        if l.lower().startswith(b'sec-websocket-key:')), None)
            if key is None:
                return None
            accept = base64.b64encode(hashlib.sha1(key + b'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest())
            self.sock.sendall(b'HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n'
                              b'Connection: Upgrade\r\nSec-WebSocket-Accept: ' + accept + b'\r\n\r\n')
            self.open = True
        text = b''
        while len(self.raw) >= 2:
            op, ln = self.raw[0] & 15, self.raw[1] & 127
            pos = 2
            if ln == 126:
                if len(self.raw) < 4:
                    break
                ln, pos = struct.unpack('>H', self.raw[2:4])[0], 4
            elif ln == 127:
                if len(self.raw) < 10:
                    break
                ln, pos = struct.unpack('>Q', self.raw[2:10])[0], 10
            masked = self.raw[1] & 128
            if len(self.raw) < pos + (4 if masked else 0) + ln:
                break
            mask = self.raw[pos:pos + 4] if masked else b'\0\0\0\0'
            pos += 4 if masked else 0
            payload = bytes(b ^ mask[i % 4] for i, b in enumerate(self.raw[pos:pos + ln]))
            self.raw = self.raw[pos + ln:]
            if op == 8:
                return None  # close
            if op == 9:  # ping
                self.sock.sendall(bytes([0x8A, len(payload)]) + payload)
            elif op in (0, 1):
                text += payload
        return text


class Bridge:
    def __init__(self, tuner_path, usb_link, watch=None):
        self.usb_link, self.watch, self.parent = usb_link, watch, os.getppid()
        self.sel = selectors.DefaultSelector()
        self.tuner = os.open(tuner_path, os.O_RDWR | os.O_NOCTTY)
        raw(self.tuner)
        self.sel.register(self.tuner, selectors.EVENT_READ, 'tuner')
        self.tuner_buf = b''

        # The USB port: we keep the far side open so it stays raw
        master, slave = pty.openpty()
        raw(slave)
        self.usb_far = slave
        os.set_blocking(master, False)  # nobody reading the USB port must not stop the bridge
        self.usb = Client('USB', fd=master)
        self.sel.register(master, selectors.EVENT_READ, self.usb)
        path = os.ttyname(slave)

        # The network ports; when another simulator has them, the next free
        # ones (the bridge reports where it is, so atutop still finds it)
        self.server = socket.socket()
        self.server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        for self.port in range(PORT, PORT + 10):
            if self.port == FIND_PORT:
                continue
            try:
                self.server.bind((LISTEN, self.port))
                break
            except OSError:
                continue
        self.server.listen(2)
        self.last_status = ''
        # The web page on 8080 and its WebSocket on 8081, or the next free
        # pair when another simulator has them
        for self.web_port in range(WEB_PORT, WEB_PORT + 20, 2):
            self.ws_port = self.web_port + 1
            self.ws_server = socket.socket()
            self.ws_server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            try:
                self.ws_server.bind((LISTEN, self.ws_port))
                self.web = http.server.ThreadingHTTPServer((LISTEN, self.web_port), self.page_handler())
                break
            except OSError:
                self.ws_server.close()
        self.ws_server.listen(4)
        self.ws_server.setblocking(False)
        self.ws_clients = []
        threading.Thread(target=self.web.serve_forever, daemon=True).start()

        try:
            os.unlink(usb_link)
        except FileNotFoundError:
            pass
        os.symlink(path, usb_link)
        print(f'USB port: {path} (linked at {usb_link}), bridge port {self.port}, web page port {self.web_port}', flush=True)
        if LAN:
            print(f'from a phone on this network: http://{ADDRESS}:{self.web_port}/', flush=True)
        self.server.setblocking(False)
        self.sel.register(self.server, selectors.EVENT_READ, 'server')
        self.sel.register(self.ws_server, selectors.EVENT_READ, 'ws_server')
        self.wifi_clients = []

        self.finder = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.finder.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            self.finder.bind(('0.0.0.0', FIND_PORT))
        except OSError:
            self.finder.bind(('0.0.0.0', 0))  # another simulator answers the search
        self.finder.setblocking(False)
        self.sel.register(self.finder, selectors.EVENT_READ, 'finder')
        self.static = None  # (ip, gateway, mask), None for the router's choice

        self.saved = None          # (ssid, password)
        self.state = 'Off'         # Off, Connecting, Connected, Failed
        self.since = time.time()
        self.hotspot = True        # nothing saved yet: hotspot for setup
        self.hotspot_always = False
        self.scan_due = []         # clients waiting for a scan

    # Wi-Fi as the real firmware does it, with pretend timings
    def tick(self):
        now = time.time()
        if self.state == 'Connecting' and now - self.since > 2:
            ssid, password = self.saved
            net = next((n for n in NETWORKS if n[0] == ssid), None)
            if net and (net[2] is None or net[2] == password):
                self.state, self.since = 'Connected', now
                self.hotspot = self.hotspot_always
                print(f'Wi-Fi: connected to {ssid}', flush=True)
            elif now - self.since > 6:
                self.state, self.since, self.hotspot = 'Failed', now, True
                print(f'Wi-Fi: cannot join {ssid}, hotspot on', flush=True)
        elif self.state == 'Failed' and now - self.since > 30:
            self.state, self.since = 'Connecting', now  # keep trying
        for client, due in list(self.scan_due):
            if now >= due:
                self.scan_due.remove((client, due))
                for ssid, rssi, password in NETWORKS:
                    self.reply(client, {'ESP32': 'Net', 'SSID': ssid, 'RSSI': rssi, 'Secure': password is not None})
                self.reply(client, {'ESP32': 'Scan', 'Count': len(NETWORKS)})
        if self.state != 'Connected':
            for c in list(self.wifi_clients):
                self.drop(c)

    def page_handler(bridge):
        """The bridge's web page, a stand-in for the real one: the tuner's
        latest status, refreshed every two seconds"""
        types = {'.html': 'text/html; charset=utf-8', '.js': 'application/javascript',
                 '.wasm': 'application/wasm', '.css': 'text/css'}

        class Page(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                # atutop's browser version, when it has been built
                name = 'index.html' if self.path in ('/', '/index.html') else self.path.lstrip('/').split('?')[0]
                path = os.path.realpath(os.path.join(WEB_DIR, name))
                if self.path != '/lite' and path.startswith(os.path.realpath(WEB_DIR)) and os.path.isfile(path):
                    with open(path, 'rb') as f:
                        body = f.read()
                    if name == 'index.html':  # tell the page where the WebSocket is
                        body = body.replace(b'</head>', f'<script>window.ATU_WS_PORT={bridge.ws_port}</script></head>'.encode())
                    self.send_response(200)
                    self.send_header('Content-Type', types.get(os.path.splitext(name)[1], 'application/octet-stream'))
                    self.send_header('Content-Length', str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                    return
                body = ('<!doctype html><meta charset="utf-8"><meta http-equiv="refresh" content="2">'
                        '<title>ATU-100 bridge (simulated)</title>'
                        '<body style="font-family:sans-serif;background:#111;color:#ddd;padding:2em">'
                        '<h1>ATU-100 bridge</h1><p>Simulated ESP32 bridge. The real one will show SWR, '
                        'power, L and C here, with Tune, Reset, Bypass and Auto buttons.</p>'
                        f'<p>Wi-Fi: {html.escape(bridge.state)}</p>'
                        f'<pre>{html.escape(bridge.last_status)}</pre></body>').encode()
                self.send_response(200)
                self.send_header('Content-Type', 'text/html; charset=utf-8')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass
        return Page

    def client_name(self, client):
        """What the bridge calls a client: a browser is on the hotspot until
        the bridge has joined a network"""
        if client is self.usb:
            return 'USB'
        if isinstance(client, WsClient) and self.state != 'Connected':
            return 'Hotspot'
        return 'WiFi'

    def reply(self, client, msg):
        client.send((json.dumps(msg, separators=(',', ':')) + '\n').encode())

    def info(self):
        connected = self.state == 'Connected'
        return {
            'ESP32': 'Info', 'Version': VERSION,
            'WiFi': self.state,
            'SSID': self.saved[0] if self.saved else '',
            'IP': ADDRESS if connected else '',
            'RSSI': next((n[1] for n in NETWORKS if self.saved and n[0] == self.saved[0]), 0) if connected else 0,
            'Host': HOST, 'Port': self.port,
            'Web': connected or self.hotspot, 'WebPort': self.web_port,
            'Hotspot': self.hotspot, 'HotspotName': HOTSPOT,
            'HotspotIP': HOTSPOT_IP if self.hotspot else '', 'HotspotAlways': self.hotspot_always,
            'StaticIP': self.static[0] if self.static else '',
            'Gateway': self.static[1] if self.static else '',
            'Mask': self.static[2] if self.static else '',
            'Clients': len(self.wifi_clients),
        }

    def command(self, client, msg):
        cmd = msg.get('ESP32')
        print(f'{client.name} asks {cmd}', flush=True)
        if cmd == 'Info':
            self.reply(client, dict(self.info(), Client=self.client_name(client)))
        elif cmd == 'Scan':
            self.scan_due.append((client, time.time() + 1.5))
        elif cmd in ('WiFi', 'Forget', 'Hotspot') and self.client_name(client) not in ('USB', 'Hotspot'):
            self.reply(client, {'ESP32': 'Error', 'Text': 'Wi-Fi settings change over USB or the hotspot only'})
        elif cmd == 'WiFi':
            ssid = str(msg.get('SSID', ''))
            if not ssid:
                self.reply(client, {'ESP32': 'Error', 'Text': 'no network name'})
                return
            if 'Pass' in msg:
                password = str(msg['Pass'])
            elif self.saved and self.saved[0] == ssid:
                password = self.saved[1]  # same network: keep its password
            else:
                password = ''
            ip = str(msg.get('IP', ''))
            if ip:
                self.static = (ip, str(msg.get('Gateway', '')), str(msg.get('Mask', '255.255.255.0')))
            elif 'IP' in msg:
                self.static = None
            self.saved = (ssid, password)
            self.state, self.since = 'Connecting', time.time()
            self.reply(client, {'ESP32': 'OK', 'Text': f'saved, joining {ssid}'})
        elif cmd == 'Hotspot':
            self.hotspot_always = bool(msg.get('Always'))
            if self.hotspot_always:
                self.hotspot = True
            elif self.state == 'Connected':
                self.hotspot = False
            self.reply(client, {'ESP32': 'OK', 'Text': 'hotspot always on' if self.hotspot_always
                                else 'hotspot only when there is no network'})
        elif cmd == 'Forget':
            self.saved, self.state, self.hotspot, self.static = None, 'Off', True, None
            self.reply(client, {'ESP32': 'OK', 'Text': 'Wi-Fi forgotten, hotspot on'})
        else:
            self.reply(client, {'ESP32': 'Error', 'Text': f'unknown command {cmd}'})

    def from_client(self, client, data):
        client.buf += data
        while b'\n' in client.buf:
            line, client.buf = client.buf.split(b'\n', 1)
            text = line.strip()
            if not text:
                continue
            try:
                msg = json.loads(text)
            except ValueError:
                msg = None
            if isinstance(msg, dict) and 'ESP32' in msg:
                self.command(client, msg)
            else:
                os.write(self.tuner, text + b'\n')  # whole lines only

    def from_tuner(self, data):
        self.tuner_buf += data
        while b'\n' in self.tuner_buf:
            line, self.tuner_buf = self.tuner_buf.split(b'\n', 1)
            if b'"SWR"' in line:
                self.last_status = line.decode(errors='replace')
            for c in [self.usb] + self.wifi_clients + self.ws_clients:
                c.send(line + b'\n')

    def drop(self, c):
        self.sel.unregister(c.sock)
        c.sock.close()
        self.wifi_clients.remove(c)
        print('Wi-Fi client gone', flush=True)

    def run(self):
        while True:
            for key, _ in self.sel.select(0.1):
                what = key.data
                if what == 'finder':
                    data, addr = self.finder.recvfrom(512)
                    try:
                        ask = json.loads(data)
                    except ValueError:
                        continue
                    if isinstance(ask, dict) and ask.get('ESP32') == 'Find' and self.state == 'Connected':
                        here = {'ESP32': 'Here', 'Version': VERSION, 'IP': ADDRESS, 'Port': self.port,
                                'Host': HOST, 'SSID': self.saved[0]}
                        self.finder.sendto(json.dumps(here, separators=(',', ':')).encode(), addr)
                elif what == 'tuner':
                    self.from_tuner(os.read(self.tuner, 4096))
                elif what == 'server':
                    sock, addr = self.server.accept()
                    if self.state != 'Connected' or len(self.wifi_clients) >= 2:
                        sock.close()
                        continue
                    c = Client('WiFi', sock=sock)
                    self.wifi_clients.append(c)
                    self.sel.register(sock, selectors.EVENT_READ, c)
                    print(f'Wi-Fi client from {addr[0]}', flush=True)
                elif what == 'ws_server':
                    sock, addr = self.ws_server.accept()
                    c = WsClient(sock)
                    self.ws_clients.append(c)
                    self.sel.register(sock, selectors.EVENT_READ, c)
                    print(f'browser from {addr[0]}', flush=True)
                elif isinstance(what, WsClient):
                    try:
                        data = what.sock.recv(4096)
                    except OSError:
                        data = b''
                    text = what.feed(data) if data else None
                    if text is None:
                        self.sel.unregister(what.sock)
                        what.sock.close()
                        self.ws_clients.remove(what)
                        print('browser gone', flush=True)
                    elif text:
                        self.from_client(what, text)
                elif what is self.usb:
                    try:
                        data = os.read(self.usb.fd, 4096)
                    except OSError:
                        data = b''
                    if data:
                        self.from_client(self.usb, data)
                else:
                    data = what.sock.recv(4096)
                    if data:
                        self.from_client(what, data)
                    else:
                        self.drop(what)
            self.tick()
            if self.watch and os.getppid() != self.parent:  # atutop is gone
                try:
                    os.kill(self.watch, signal.SIGTERM)
                except OSError:
                    pass
                try:
                    os.unlink(self.usb_link)
                except OSError:
                    pass
                return


if __name__ == '__main__':
    args = sys.argv[1:]
    watch = None
    if '--watch' in args:
        i = args.index('--watch')
        watch = int(args[i + 1])
        del args[i:i + 2]
    if not args:
        sys.exit(__doc__)
    bridge = Bridge(args[0], args[1] if len(args) > 1 else '/tmp/atu100-esp32', watch)

    def stop(*_):
        try:
            os.unlink(bridge.usb_link)
        except OSError:
            pass
        sys.exit(0)
    for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
        signal.signal(sig, stop)
    bridge.run()
