#!/usr/bin/env python3
"""A simulated ESP32 bridge, for trying atutop's settings without the board.

    ./esp32_sim.py TUNER_PORT [USB_LINK] [--watch TUNER_PID]

TUNER_PORT is the simulated tuner's pseudo terminal (host_test with PTY=1).
This makes the bridge's "USB" port, a pseudo terminal linked at USB_LINK
(default /tmp/atu100-esp32), and once "connected" to Wi-Fi a TCP bridge on
127.0.0.1:7000, which stands in for the ESP32's address on the network.

It behaves as the real bridge firmware does (README.md, "ESP32 bridge"):
lines are passed whole between the tuner and every client; lines with an
"ESP32" field are answered here and never reach the tuner; Wi-Fi settings
change over USB only; {"ESP32":"Find"} on UDP port 7001 is answered with
where to connect. A fixed IP address is kept and reported, but the bridge
stays on 127.0.0.1 here. Pretend networks: VE7NHW-shack (password hamradio),
two others with unknown passwords, and an open one.

--watch: started by atutop, which passes the simulated tuner's process id.
When atutop goes (even without quitting properly) this stops the tuner,
removes USB_LINK and ends.
"""

import html
import http.server
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
HOST = 'atu100.local'
HOTSPOT = 'ATU100-setup'
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
                self.server.bind(('127.0.0.1', self.port))
                break
            except OSError:
                continue
        self.server.listen(2)
        self.last_status = ''
        self.web = http.server.ThreadingHTTPServer(('127.0.0.1', 0), self.page_handler())
        self.web_port = self.web.server_address[1]
        threading.Thread(target=self.web.serve_forever, daemon=True).start()

        try:
            os.unlink(usb_link)
        except FileNotFoundError:
            pass
        os.symlink(path, usb_link)
        print(f'USB port: {path} (linked at {usb_link}), bridge port {self.port}, web page port {self.web_port}', flush=True)
        self.server.setblocking(False)
        self.sel.register(self.server, selectors.EVENT_READ, 'server')
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
        self.scan_due = []         # clients waiting for a scan

    # Wi-Fi as the real firmware does it, with pretend timings
    def tick(self):
        now = time.time()
        if self.state == 'Connecting' and now - self.since > 2:
            ssid, password = self.saved
            net = next((n for n in NETWORKS if n[0] == ssid), None)
            if net and (net[2] is None or net[2] == password):
                self.state, self.since, self.hotspot = 'Connected', now, False
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
        class Page(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
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

    def reply(self, client, msg):
        client.send((json.dumps(msg, separators=(',', ':')) + '\n').encode())

    def info(self):
        connected = self.state == 'Connected'
        return {
            'ESP32': 'Info', 'Version': VERSION,
            'WiFi': self.state,
            'SSID': self.saved[0] if self.saved else '',
            'IP': '127.0.0.1' if connected else '',
            'RSSI': next((n[1] for n in NETWORKS if self.saved and n[0] == self.saved[0]), 0) if connected else 0,
            'Host': HOST, 'Port': self.port,
            'Web': connected, 'WebPort': self.web_port,
            'Hotspot': self.hotspot, 'HotspotName': HOTSPOT,
            'StaticIP': self.static[0] if self.static else '',
            'Gateway': self.static[1] if self.static else '',
            'Mask': self.static[2] if self.static else '',
            'Clients': len(self.wifi_clients),
        }

    def command(self, client, msg):
        cmd = msg.get('ESP32')
        print(f'{client.name} asks {cmd}', flush=True)
        if cmd == 'Info':
            self.reply(client, dict(self.info(), Client=client.name))
        elif cmd == 'Scan':
            self.scan_due.append((client, time.time() + 1.5))
        elif cmd in ('WiFi', 'Forget') and client is not self.usb:
            self.reply(client, {'ESP32': 'Error', 'Text': 'Wi-Fi settings change over USB only'})
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
            for c in [self.usb] + self.wifi_clients:
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
                        here = {'ESP32': 'Here', 'Version': VERSION, 'IP': '127.0.0.1', 'Port': self.port,
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
