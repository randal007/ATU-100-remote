/*  atu100_bridge: ESP32-S3 bridge for the ATU-100 EXT tuner running the
    display-buttons-uart firmware. Written for the Seeed XIAO ESP32S3 Plus.

    - USB: everything passes between the USB port and the tuner, so atutop
      and the Python tools work as with a USB serial adapter.
    - Wi-Fi: the same over TCP port 7000 (socket://atu100.local:7000), up to
      two clients; a web page on port 80; answers {"ESP32":"Find"} on UDP
      port 7001 so atutop can find it.
    - An Arduino Mega on a second serial port, as if wired to the tuner.
    - Its own messages carry an "ESP32" field and never reach the tuner
      (see README.md, "ESP32 bridge"; test/esp32_sim.py does the same on a
      PC). Wi-Fi settings change over USB only, or from the setup page on
      its own hotspot "ATU100-setup" (password hamradio, address
      192.168.73.1), which comes up when it has no network or cannot join
      it, or stays on always if set so. Everything works on the hotspot
      too, for use off-grid with no other network.

    The hotspot and the home network share one radio, so the hotspot
    follows the home network's channel: joining a network moves it, and
    anything on the hotspot drops for a moment. So it does not try the
    home network again while someone is on the hotspot.

    Lines are passed whole: messages from different sides never mix.

    Wiring (XIAO pin names):
      D7 (GPIO44) RX  <- tuner RA7, PIC pin 9, through 2.2k / 3.3k (5 V -> 3.3 V)
      D6 (GPIO43) TX  -> tuner RA6, PIC pin 10 (10k pull-up there to 3.3 V)
      D8 (GPIO7)  RX  <- Mega TX1 (pin 18), through 2.2k / 3.3k
      D9 (GPIO8)  TX  -> Mega RX1 (pin 19)
      5V <- a 7805 from the tuner's 12 V; GND to the tuner's and the Mega's
*/

#include <ArduinoJson.h>
#include <ESPmDNS.h>
#include <Preferences.h>
#include <WebServer.h>
#include <WiFi.h>
#include <WiFiUdp.h>

#define VERSION "1.0"
#define HOST "atu100" // atu100.local
#define HOTSPOT "ATU100-setup"
#define HOTSPOT_PASS "hamradio" // at least 8 characters for WPA2
const IPAddress HOTSPOT_IP(192, 168, 73, 1);
#define BRIDGE_PORT 7000
#define FIND_PORT 7001
#define WEB_PORT 80
#define TUNER_BAUD 4800
#define TUNER_RX 44
#define TUNER_TX 43
#define MEGA_RX 7
#define MEGA_TX 8
#define MAX_TCP 2
#define LINE_MAX 300
#define JOIN_TIMEOUT 20000  // ms to join a network before the hotspot comes up
#define RETRY_EVERY 30000   // then try the network again this often
#define HOTSPOT_LINGER 30000 // keep the hotspot this long after joining

HardwareSerial &tuner = Serial1;
HardwareSerial &mega = Serial2;
WiFiServer bridge(BRIDGE_PORT);
WiFiClient tcp[MAX_TCP];
WebServer web(WEB_PORT);
WiFiUDP finder;
Preferences prefs;

enum Who { USB, MEGA, TCP0 }; // TCP0 + i for the TCP clients

struct LineBuf {
  char buf[LINE_MAX];
  size_t n = 0;
  bool overflow = false;
  // Add a character; true when a whole line is ready in buf
  bool add(char c) {
    if (c == '\r')
      return false;
    if (c == '\n') {
      bool ready = n > 0 && !overflow;
      buf[n] = 0;
      if (!ready)
        n = 0;
      overflow = false;
      return ready;
    }
    if (n < LINE_MAX - 1)
      buf[n++] = c;
    else
      overflow = true; // too long: dropped whole
    return false;
  }
};

LineBuf usbLine, megaLine, tunerLine, tcpLine[MAX_TCP];

// Wi-Fi
enum WifiState { W_OFF, W_CONNECTING, W_CONNECTED, W_FAILED };
WifiState wifiState = W_OFF;
const char *wifiName[] = {"Off", "Connecting", "Connected", "Failed"};
String ssid, pass, staticIp, gateway, mask;
uint32_t wifiSince = 0, retryAt = 0, hotspotOffAt = 0;
bool hotspot = false, hotspotAlways = false;
int scanFor = -1; // who asked for a scan in progress

// The tuner, for the web page
String lastStatus;
String events[8];
uint8_t eventsIn = 0;
bool tuning = false;

// ---------------------------------------------------------------- sending

void sendTo(int who, const char *line) {
  if (who == USB) {
    Serial.print(line);
    Serial.print('\n');
  } else if (who == MEGA) {
    mega.print(line);
    mega.print('\n');
  } else if (tcp[who - TCP0].connected()) {
    tcp[who - TCP0].print(line);
    tcp[who - TCP0].print('\n');
  }
}

void reply(int who, JsonDocument &doc) {
  String out;
  serializeJson(doc, out);
  sendTo(who, out.c_str());
}

void notice(int who, const char *kind, const String &text) {
  JsonDocument d;
  d["ESP32"] = kind;
  d["Text"] = text;
  reply(who, d);
}

void toTuner(const char *line) {
  tuner.print(line);
  tuner.print('\n');
}

// --------------------------------------------------------------- Wi-Fi

bool haveStatic() { return staticIp.length() > 0; }

void startHotspot() {
  if (hotspot)
    return;
  WiFi.mode(ssid.length() ? WIFI_AP_STA : WIFI_AP);
  WiFi.softAPConfig(HOTSPOT_IP, HOTSPOT_IP, IPAddress(255, 255, 255, 0));
  WiFi.softAP(HOTSPOT, HOTSPOT_PASS);
  hotspot = true;
}

void stopHotspot() {
  if (!hotspot || hotspotAlways)
    return;
  WiFi.softAPdisconnect(true);
  WiFi.mode(WIFI_STA);
  hotspot = false;
}

void startJoin() {
  if (!ssid.length()) {
    wifiState = W_OFF;
    WiFi.disconnect();
    startHotspot();
    return;
  }
  WiFi.mode(hotspot ? WIFI_AP_STA : WIFI_STA);
  WiFi.setHostname(HOST);
  IPAddress ip, gw, nm;
  if (haveStatic() && ip.fromString(staticIp) && gw.fromString(gateway) && nm.fromString(mask))
    WiFi.config(ip, gw, nm, gw); // the router as DNS too
  else
    WiFi.config(INADDR_NONE, INADDR_NONE, INADDR_NONE); // DHCP
  WiFi.begin(ssid.c_str(), pass.c_str());
  wifiState = W_CONNECTING;
  wifiSince = millis();
}

void loadSettings() {
  prefs.begin("atu100", true);
  ssid = prefs.getString("ssid", "");
  pass = prefs.getString("pass", "");
  staticIp = prefs.getString("ip", "");
  gateway = prefs.getString("gw", "");
  mask = prefs.getString("mask", "");
  hotspotAlways = prefs.getBool("apAlways", false);
  prefs.end();
}

void saveSettings() {
  prefs.begin("atu100", false);
  prefs.putString("ssid", ssid);
  prefs.putString("pass", pass);
  prefs.putString("ip", staticIp);
  prefs.putString("gw", gateway);
  prefs.putString("mask", mask);
  prefs.putBool("apAlways", hotspotAlways);
  prefs.end();
}

void wifiLoop() {
  uint32_t now = millis();
  bool up = WiFi.status() == WL_CONNECTED;
  switch (wifiState) {
  case W_CONNECTING:
    if (up) {
      wifiState = W_CONNECTED;
      if (hotspot && !hotspotAlways)
        hotspotOffAt = now + HOTSPOT_LINGER; // time for the setup page to say so
    } else if (now - wifiSince > JOIN_TIMEOUT) {
      wifiState = W_FAILED;
      startHotspot();
      retryAt = now + RETRY_EVERY;
    }
    break;
  case W_CONNECTED:
    if (!up) { // lost it: the ESP32 tries again by itself
      wifiState = W_CONNECTING;
      wifiSince = now;
    } else if (hotspot && hotspotOffAt && (int32_t)(now - hotspotOffAt) > 0 && WiFi.softAPgetStationNum() == 0) {
      stopHotspot();
      hotspotOffAt = 0;
    }
    break;
  case W_FAILED:
    if (up)
      wifiState = W_CONNECTING; // joined after all
    else if ((int32_t)(now - retryAt) > 0) {
      retryAt = now + RETRY_EVERY;
      if (WiFi.softAPgetStationNum() == 0) // not while someone uses the hotspot
        startJoin();
    }
    break;
  case W_OFF:
    break;
  }
}

// ------------------------------------------------- the bridge's own messages

const char *whoName(int who) { return who == USB ? "USB" : who == MEGA ? "Mega" : "WiFi"; }

void sendInfo(int who) {
  JsonDocument d;
  bool up = wifiState == W_CONNECTED;
  d["ESP32"] = "Info";
  d["Version"] = VERSION;
  d["Client"] = whoName(who);
  d["WiFi"] = wifiName[wifiState];
  d["SSID"] = ssid;
  d["IP"] = up ? WiFi.localIP().toString() : String("");
  d["RSSI"] = up ? WiFi.RSSI() : 0;
  d["Host"] = HOST ".local";
  d["Port"] = BRIDGE_PORT;
  d["Web"] = up || hotspot; // served on every network it is on
  d["WebPort"] = WEB_PORT;
  d["Hotspot"] = hotspot;
  d["HotspotName"] = HOTSPOT;
  d["HotspotIP"] = hotspot ? HOTSPOT_IP.toString() : String("");
  d["HotspotAlways"] = hotspotAlways;
  d["StaticIP"] = staticIp;
  d["Gateway"] = gateway;
  d["Mask"] = mask;
  int n = 0;
  for (int i = 0; i < MAX_TCP; i++)
    n += tcp[i].connected();
  d["Clients"] = n;
  reply(who, d);
}

// Apply new Wi-Fi settings; an empty message on success, else what is wrong
String applyWifi(const String &newSsid, bool havePass, const String &newPass, bool haveIp, const String &ip,
                 const String &gw, const String &nm) {
  if (!newSsid.length() || newSsid.length() > 32)
    return "no network name";
  if (havePass && newPass.length() > 63)
    return "password too long";
  if (haveIp && ip.length()) {
    IPAddress a, b, c;
    if (!a.fromString(ip) || !b.fromString(gw) || !c.fromString(nm))
      return "the IP address, gateway or mask is not valid";
    staticIp = ip;
    gateway = gw;
    mask = nm;
  } else if (haveIp) {
    staticIp = gateway = mask = ""; // the router's choice
  }
  if (havePass)
    pass = newPass;
  else if (newSsid != ssid)
    pass = ""; // a new network without a password: an open one
  ssid = newSsid;
  saveSettings();
  WiFi.disconnect();
  startJoin();
  return "";
}

void command(int who, JsonDocument &doc) {
  String cmd = doc["ESP32"] | "";
  if (cmd == "Info") {
    sendInfo(who);
  } else if (cmd == "Scan") {
    if (scanFor < 0 && WiFi.scanNetworks(true) == WIFI_SCAN_FAILED)
      notice(who, "Error", "scan failed");
    else
      scanFor = who;
  } else if ((cmd == "WiFi" || cmd == "Forget" || cmd == "Hotspot") && who != USB) {
    notice(who, "Error", "Wi-Fi settings change over USB only");
  } else if (cmd == "WiFi") {
    String err = applyWifi(doc["SSID"] | "", doc["Pass"].is<const char *>(), doc["Pass"] | "",
                           doc["IP"].is<const char *>(), doc["IP"] | "", doc["Gateway"] | "",
                           doc["Mask"] | "255.255.255.0");
    if (err.length())
      notice(who, "Error", err);
    else
      notice(who, "OK", "saved, joining " + ssid);
  } else if (cmd == "Hotspot") {
    hotspotAlways = doc["Always"] | false;
    saveSettings();
    if (hotspotAlways)
      startHotspot();
    else if (wifiState == W_CONNECTED)
      stopHotspot();
    notice(who, "OK", hotspotAlways ? "hotspot always on" : "hotspot only when there is no network");
  } else if (cmd == "Forget") {
    ssid = pass = staticIp = gateway = mask = "";
    saveSettings();
    WiFi.disconnect();
    wifiState = W_OFF;
    startHotspot();
    notice(who, "OK", "Wi-Fi forgotten, hotspot on");
  } else {
    notice(who, "Error", "unknown command " + cmd);
  }
}

void scanLoop() {
  if (scanFor < 0)
    return;
  int n = WiFi.scanComplete();
  if (n == WIFI_SCAN_RUNNING)
    return;
  if (n < 0) {
    notice(scanFor, "Error", "scan failed");
  } else {
    for (int i = 0; i < n; i++) {
      JsonDocument d;
      d["ESP32"] = "Net";
      d["SSID"] = WiFi.SSID(i);
      d["RSSI"] = WiFi.RSSI(i);
      d["Secure"] = WiFi.encryptionType(i) != WIFI_AUTH_OPEN;
      reply(scanFor, d);
    }
    JsonDocument d;
    d["ESP32"] = "Scan";
    d["Count"] = n;
    reply(scanFor, d);
  }
  WiFi.scanDelete();
  scanFor = -1;
}

// A whole line from a client: the bridge's own, or for the tuner
void fromClient(int who, const char *line) {
  if (strstr(line, "\"ESP32\"")) {
    JsonDocument doc;
    if (!deserializeJson(doc, line) && doc["ESP32"].is<const char *>()) {
      command(who, doc);
      return;
    }
  }
  // Only JSON from the Mega and the network: a floating or noisy line
  // must not send the single letters the tuner takes as button presses
  if (who != USB && line[0] != '{')
    return;
  toTuner(line);
}

// A whole line from the tuner: to everyone, and kept for the web page
void fromTuner(const char *line) {
  sendTo(USB, line);
  sendTo(MEGA, line);
  for (int i = 0; i < MAX_TCP; i++)
    sendTo(TCP0 + i, line);
  if (strstr(line, "\"SWR\""))
    lastStatus = line;
  if (strstr(line, "\"Event\"")) {
    events[eventsIn++ % 8] = line;
    if (strstr(line, "\"Tune\"}"))
      tuning = true;
    else if (strstr(line, "\"Tuned\"") || strstr(line, "\"Reset\"") || strstr(line, "\"Startup\""))
      tuning = false;
  }
}

// ----------------------------------------------------------------- web page

const char PAGE[] PROGMEM = R"HTML(<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>ATU-100</title>
<style>
:root{--bg:#111;--box:#1a1a1a;--fg:#ccc;--hi:#eee;--dim:#777;--g:#50f095;--y:#f2e266;--r:#fa1e1e;--on:#2d6b3c}
body{margin:0;font:16px system-ui,sans-serif;background:var(--bg);color:var(--fg)}
main{max-width:520px;margin:auto;padding:16px}
h1{font-size:20px;color:var(--hi);margin:0 0 12px}h1 b{color:#b54040}
.box{background:var(--box);border:1px solid #333;border-radius:10px;padding:12px;margin-bottom:12px}
.big{font-size:40px;color:var(--hi);font-weight:600}.row{display:flex;justify-content:space-between;gap:8px}
.bar{height:10px;background:#3a3a3a;border-radius:5px;overflow:hidden;margin-top:6px}
.bar i{display:block;height:100%;background:linear-gradient(90deg,var(--g),var(--y),var(--r))}
.dim{color:var(--dim);font-size:13px}button{flex:1;font:inherit;padding:14px 0;border:0;border-radius:8px;
background:#303030;color:var(--hi)}button.on{background:var(--on)}.alarm{color:var(--r);font-weight:600}
#ev div{font-size:13px;color:var(--dim)}a{color:var(--g)}
</style></head><body><main>
<h1>atu<b>100</b> <span class="dim" id="tun"></span></h1>
<div class="row"><div class="box" style="flex:1"><div class="dim">power</div><div class="big" id="pwr">-</div>
<div class="dim" id="ant"></div><div class="bar"><i id="pb" style="width:0"></i></div></div>
<div class="box" style="flex:1"><div class="dim">swr</div><div class="big" id="swr">-</div>
<div class="dim" id="al"></div><div class="bar"><i id="sb" style="width:0"></i></div></div></div>
<div class="box"><div class="row"><span>L <b id="l">-</b> nH</span><span>C <b id="c">-</b> pF</span>
<span id="o"></span></div></div>
<div class="row" style="margin-bottom:12px"><button onclick="cmd('tune')">Tune</button>
<button onclick="cmd('reset')">Reset</button><button id="by" onclick="cmd('bypass')">Bypass</button>
<button id="au" onclick="cmd('auto')">Auto</button></div>
<div class="box" id="ev"></div><div class="dim" id="setup"></div>
</main><script>
let st={};
function cmd(c){fetch('/cmd?c='+c,{method:'POST'})}
async function poll(){try{const r=await (await fetch('/status')).json();st=r.Tuner||{};
const p=st.Power??0,s=st.SWR??0;pwr.textContent=p.toFixed(1)+' W';
ant.textContent=st.Antenna!=null?'antenna '+st.Antenna.toFixed(1)+' W  '+st.Efficiency+'%':'';
swr.textContent=s?s.toFixed(2):'-.--';pb.style.width=Math.min(100,p/r.Scale*100)+'%';
sb.style.width=s?Math.min(100,(s-1)/2*100)+'%':'0';l.textContent=st.Inductance??'-';
c.textContent=st.Capacitance??'-';o.textContent=st.Order=='LC'?'LC  C at antenna':st.Order?'CL  C at radio':'';
by.className=st.Bypass?'on':'';au.className=st.Auto?'on':'';tun.textContent=r.Tuning?'TUNING · transmit a carrier':'';
ev.innerHTML=r.Events.map(e=>'<div>'+(e.Event||'')+'</div>').join('');
setup.innerHTML=r.Hotspot?'Hotspot on: <a href="/setup">Wi-Fi setup</a>':'';}catch(e){}}
setInterval(poll,1000);poll();
</script></body></html>)HTML";

// Requests that arrived over the bridge's own hotspot may change Wi-Fi
bool viaHotspot() { return hotspot && web.client().localIP() == WiFi.softAPIP(); }

void webStatus() {
  String out = "{\"Tuner\":";
  out += lastStatus.length() ? lastStatus : String("null");
  out += ",\"Tuning\":";
  out += tuning ? "true" : "false";
  out += ",\"Hotspot\":";
  out += viaHotspot() ? "true" : "false";
  out += ",\"Scale\":100,\"Events\":[";
  bool first = true;
  for (int i = 0; i < 8; i++) { // newest first
    const String &e = events[(eventsIn + 7 - i) % 8];
    if (!e.length())
      continue;
    if (!first)
      out += ',';
    out += e;
    first = false;
  }
  out += "]}";
  web.send(200, "application/json", out);
}

void webCommand() {
  String c = web.arg("c");
  if (c == "tune")
    toTuner("{\"Tune\": true}");
  else if (c == "reset")
    toTuner("{\"Reset\": true}");
  else if (c == "bypass")
    toTuner(lastStatus.indexOf("\"Bypass\":true") >= 0 ? "{\"Bypass\": false}" : "{\"Bypass\": true}");
  else if (c == "auto")
    toTuner(lastStatus.indexOf("\"Auto\":true") >= 0 ? "{\"Auto\": false}" : "{\"Auto\": true}");
  else {
    web.send(400, "text/plain", "unknown command");
    return;
  }
  web.send(204);
}

String htmlEscape(const String &s) {
  String o;
  for (char ch : s) {
    if (ch == '<') o += "&lt;";
    else if (ch == '>') o += "&gt;";
    else if (ch == '&') o += "&amp;";
    else if (ch == '"') o += "&quot;";
    else o += ch;
  }
  return o;
}

void webSetup() {
  if (!viaHotspot()) {
    web.send(403, "text/plain", "Wi-Fi setup only over the bridge's own hotspot, or with atutop over USB");
    return;
  }
  if (web.method() == HTTP_POST) {
    bool fixed = web.arg("mode") == "fixed";
    String err = applyWifi(web.arg("ssid"), web.arg("pass").length() > 0 || web.arg("ssid") != ssid, web.arg("pass"),
                           true, fixed ? web.arg("ip") : String(""), web.arg("gw"), web.arg("mask"));
    String body = "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'>"
                  "<body style='font-family:sans-serif;background:#111;color:#ccc;padding:2em'>";
    if (err.length())
      body += "<p style='color:#fa1e1e'>" + htmlEscape(err) + "</p><p><a href='/setup'>Back</a></p>";
    else
      body += "<p>Saved. Joining <b>" + htmlEscape(ssid) + "</b>&hellip;</p><p>" +
              String(hotspotAlways ? "The hotspot stays on. " : "The hotspot closes about half a minute after the "
                                                                 "bridge is on your network. ") +
              "On your network, open <b>http://atu100.local</b>, or use atutop's connect screen.</p>";
    web.send(200, "text/html", body);
    return;
  }
  int n = WiFi.scanNetworks(); // a few seconds; fine for a setup page
  String opts;
  for (int i = 0; i < n; i++)
    opts += "<option value=\"" + htmlEscape(WiFi.SSID(i)) + "\">" + htmlEscape(WiFi.SSID(i)) + " (" +
            String(WiFi.RSSI(i)) + " dBm)</option>";
  WiFi.scanDelete();
  String body =
      "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'>"
      "<title>ATU-100 Wi-Fi</title><body style='font:16px sans-serif;background:#111;color:#ccc;padding:1em'>"
      "<h2>ATU-100 bridge Wi-Fi</h2><form method=post>"
      "<p>Network<br><input name=ssid list=nets value=\"" + htmlEscape(ssid) + "\" required>"
      "<datalist id=nets>" + opts + "</datalist></p>"
      "<p>Password<br><input name=pass type=password placeholder='empty keeps the saved one'></p>"
      "<p>IP address<br><label><input type=radio name=mode value=dhcp" + String(haveStatic() ? "" : " checked") +
      "> from the router</label> <label><input type=radio name=mode value=fixed" + String(haveStatic() ? " checked" : "") +
      "> fixed</label></p>"
      "<p>Address <input name=ip value=\"" + htmlEscape(staticIp) + "\" placeholder=192.168.1.60><br>"
      "Gateway <input name=gw value=\"" + htmlEscape(gateway) + "\" placeholder=192.168.1.1><br>"
      "Mask <input name=mask value=\"" + htmlEscape(mask.length() ? mask : String("255.255.255.0")) + "\"></p>"
      "<p><button>Save and connect</button></p></form><p><a href='/' style='color:#50f095'>Tuner page</a></p>";
  web.send(200, "text/html", body);
}

// ------------------------------------------------------------ setup / loop

void setup() {
  Serial.begin(115200);    // USB: the speed does not matter
  Serial.setTxTimeoutMs(0); // never wait for a computer that is not there
  tuner.begin(TUNER_BAUD, SERIAL_8N1, TUNER_RX, TUNER_TX);
  mega.begin(TUNER_BAUD, SERIAL_8N1, MEGA_RX, MEGA_TX);
  pinMode(TUNER_RX, INPUT_PULLUP); // quiet when nothing is connected
  pinMode(MEGA_RX, INPUT_PULLUP);

  loadSettings();
  WiFi.persistent(false); // our settings live in Preferences
  WiFi.setAutoReconnect(true);
  if (hotspotAlways)
    startHotspot();
  startJoin(); // or the hotspot, with nothing saved
  // atu100.local on the hotspot and the home network alike
  if (MDNS.begin(HOST)) {
    MDNS.addService("http", "tcp", WEB_PORT);
    MDNS.addService("atu100", "tcp", BRIDGE_PORT);
  }

  bridge.begin();
  bridge.setNoDelay(true);
  finder.begin(FIND_PORT);
  web.on("/", [] { web.send_P(200, "text/html", PAGE); });
  web.on("/status", webStatus);
  web.on("/cmd", HTTP_POST, webCommand);
  web.on("/setup", webSetup);
  web.begin();
}

void loop() {
  while (Serial.available())
    if (usbLine.add(Serial.read()))
      fromClient(USB, usbLine.buf), usbLine.n = 0;
  while (mega.available())
    if (megaLine.add(mega.read()))
      fromClient(MEGA, megaLine.buf), megaLine.n = 0;
  while (tuner.available())
    if (tunerLine.add(tuner.read()))
      fromTuner(tunerLine.buf), tunerLine.n = 0;

  // TCP clients: new ones into a free place, lines from each
  WiFiClient c = bridge.accept();
  if (c) {
    int free = -1;
    for (int i = 0; i < MAX_TCP; i++)
      if (!tcp[i].connected())
        free = i;
    if (free < 0)
      c.stop();
    else {
      tcp[free] = c;
      tcp[free].setNoDelay(true);
      tcpLine[free].n = 0;
    }
  }
  for (int i = 0; i < MAX_TCP; i++)
    while (tcp[i].connected() && tcp[i].available())
      if (tcpLine[i].add(tcp[i].read()))
        fromClient(TCP0 + i, tcpLine[i].buf), tcpLine[i].n = 0;

  // "Who is there?" from atutop's connect screen
  int len = finder.parsePacket();
  if (len > 0) {
    char ask[64];
    int n = finder.read(ask, sizeof ask - 1);
    ask[n > 0 ? n : 0] = 0;
    bool viaAp = hotspot && (finder.remoteIP() & IPAddress(255, 255, 255, 0)) == (WiFi.softAPIP() & IPAddress(255, 255, 255, 0));
    if (strstr(ask, "\"Find\"") && (wifiState == W_CONNECTED || viaAp)) {
      JsonDocument d;
      d["ESP32"] = "Here";
      d["Version"] = VERSION;
      d["IP"] = viaAp ? WiFi.softAPIP().toString() : WiFi.localIP().toString();
      d["Port"] = BRIDGE_PORT;
      d["Host"] = HOST ".local";
      d["SSID"] = viaAp ? String(HOTSPOT) : ssid;
      String out;
      serializeJson(d, out);
      finder.beginPacket(finder.remoteIP(), finder.remotePort());
      finder.print(out);
      finder.endPacket();
    }
  }

  wifiLoop();
  scanLoop();
  web.handleClient();
}
