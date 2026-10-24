/*  ATU-100 bridge for an Arduino Mega

    Tuner on Serial1 (pin 19 RX1 <- tuner RA7, pin 18 TX1 -> tuner RA6,
    10k pull-up on RA6), PC on the USB port.

    - Passes everything both ways, so the python tools in this repository
      work through the Mega's USB port.
    - Keeps the latest SWR, power and relay values from the tuner's status
      lines, for the rest of the amplifier code to use.
    - TUNING_PIN is high from the tuner's "Tune" event to its "Tuned"
      event: the Tx_req signal the tuner used to output itself. Use it to
      key a low power carrier and keep the amplifier in bypass.
    - atuHighSwr is set by the tuner's "HighSWR" event (sent within a few
      ms of the SWR passing the limit in eeprom cell 0x37) and cleared
      when a status line shows the SWR back under 2.
*/

const long ATU_BAUD = 4800;
const int TUNING_PIN = 22;

float atuSwr = 0, atuPower = 0;
long atuFwdMv = 0, atuRevMv = 0;     // from atuRaw()
bool atuHighSwr = false;
long atuInductance = 0, atuCapacitance = 0; // nH, pF
bool atuTuning = false, atuBypass = false, atuAuto = false;

char line[200];
int lineLen = 0;

// Value after "name": in a status line, or NAN when it is not there
float field(const char *name)
{
  char key[24];
  snprintf(key, sizeof key, "\"%s\":", name);
  const char *p = strstr(line, key);
  return p ? atof(p + strlen(key)) : NAN;
}

bool flag(const char *name, bool current)
{
  char key[24];
  snprintf(key, sizeof key, "\"%s\":", name);
  const char *p = strstr(line, key);
  return p ? p[strlen(key)] == 't' : current;
}

void atuLine()
{
  if (strstr(line, "\"Event\":\"Tune\"")) {
    atuTuning = true;
    digitalWrite(TUNING_PIN, HIGH);
  } else if (strstr(line, "\"Event\":\"Tuned\"") || strstr(line, "\"Event\":\"Reset\"")) {
    atuTuning = false;
    digitalWrite(TUNING_PIN, LOW);
  } else if (strstr(line, "\"Event\":\"HighSWR\"")) {
    atuHighSwr = true; // e.g. drop the amplifier to bypass here
  } else if (strstr(line, "\"Fwd\":")) {
    atuFwdMv = (long)field("Fwd");
    atuRevMv = (long)field("Rev");
  } else if (strstr(line, "\"SWR\":")) {
    atuSwr = field("SWR");
    atuPower = field("Power");
    atuInductance = (long)field("Inductance");
    atuCapacitance = (long)field("Capacitance");
    atuBypass = flag("Bypass", atuBypass);
    atuAuto = flag("Auto", atuAuto);
    if (atuSwr < 2.0)
      atuHighSwr = false;
  }
}

// Commands for the tuner. Wait for the tuner's reply before sending the next.
void atuTune() { Serial1.print("{\"Tune\":true}\n"); }
void atuReset() { Serial1.print("{\"Reset\":true}\n"); }
void atuStatus() { Serial1.print("{\"Status\":true}\n"); }
void atuRaw() { Serial1.print("{\"Raw\":true}\n"); }
void atuSetBypass(bool on) { Serial1.print(on ? "{\"Bypass\":true}\n" : "{\"Bypass\":false}\n"); }

// Relays directly: inductors and capacitors as bit sets, C on the output side if cOut
void atuSetRelays(byte l, byte c, bool cOut)
{
  Serial1.print("{\"RelayI\":");
  Serial1.print(l);
  Serial1.print(",\"RelayC\":");
  Serial1.print(c | (cOut ? 0x80 : 0));
  Serial1.print("}\n");
}

void setup()
{
  pinMode(TUNING_PIN, OUTPUT);
  digitalWrite(TUNING_PIN, LOW);
  Serial.begin(115200);
  Serial1.begin(ATU_BAUD);
}

void loop()
{
  while (Serial1.available()) {
    char c = Serial1.read();
    Serial.write(c);
    if (c == '\n') {
      line[lineLen] = 0;
      atuLine();
      lineLen = 0;
    } else if (lineLen < (int)sizeof line - 1) {
      line[lineLen++] = c;
    }
  }
  while (Serial.available())
    Serial1.write(Serial.read());
}
