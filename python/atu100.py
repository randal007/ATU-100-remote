#!/usr/bin/env python3
'''
ATU-100 control program and library

When called as a program provides a simple traditional command line control
of a remote ATU-100.

The ATU-100 needs the firmware from
https://github.com/randal007/ATU-100-remote (branch display-buttons-uart),
with the 5 V TTL serial lines on PIC pins 9 and 10 wired to this computer,
or reached through an Arduino or ESP32 bridge.

This allows the exchange of JSON format messages to control the ATU-100 and
show its status.  The messages are documented in README.md in that repository.

The port can be a serial device (/dev/ttyACM0, COM3) or any pyserial URL,
such as socket://atu100.local:7000 for a network bridge.
'''

import argparse
import datetime
import json
import logging
import os
import serial
import string
import sys

from datetime import datetime
from enum import Enum
from time import sleep, time



# Events shown in red by the remote and the diagnostics
BAD_EVENTS = ('HighSWR', 'HighPower', 'Overload', 'Error')


class cell:
    EEPROM_DISPLAY_I2C_ADDR         = 0x00
    EEPROM_DISPLAY_TYPE             = 0x01
    EEPROM_AUTOMATIC_MODE           = 0x02
    EEPROM_TIMEOUT_TIME             = 0x03
    EEPROM_SWR_THRESHOLD            = 0x04
    EEPROM_MIN_POWER                = 0x05
    EEPROM_MAX_POWER                = 0x06
    EEPROM_DISPLAY_OFFSET_DOWN      = 0x07
    EEPROM_DISPLAY_OFFSET_LEFT      = 0x08
    EEPROM_MAX_INIT_SWR             = 0x09
    EEPROM_NUMBER_INDS              = 0x0a
    EEPROM_IND_LINEAR_PITCH         = 0x0b
    EEPROM_NUMBER_CAPS              = 0x0c
    EEPROM_CAP_LINEAR_PITCH         = 0x0d
    EEPROM_ENABLE_NONLINEAR_DIODE   = 0x0e
    EEPROM_INVERSE_INDUCTANCE_RELAY = 0x0f

    EEPROM_INDUCTOR_FIRST           = 0x10
    EEPROM_INDUCTOR_LAST            = 0x1D
    
    EEPROM_CAPACITOR_FIRST          = 0x20
    EEPROM_CAPACITOR_LAST           = 0x2D

    EEPROM_POWER_MEASURE_LEVEL      = 0x30
    EEPROM_TANDEM_MATCH             = 0x31
    EEPROM_DISPLAY_OFF_TIMER        = 0x32
    EEPROM_ADDITIONAL_INDICATION    = 0x33
    EEPROM_FEEDER_LOSS              = 0x34
    EEPROM_DISABLE_RELAYS           = 0x35
    EEPROM_SWR_ALARM                = 0x37  # HighSWR limit, BCD SWR x 10, 0 = off, ff = 3.0

    EEPROM_LAST_SWR_L               = 0xfb
    EEPROM_LAST_SWR_H               = 0xfc
    EEPROM_LAST_SW                  = 0xfd
    EEPROM_LAST_IND                 = 0xfe
    EEPROM_LAST_CAP                 = 0xff



class atu100(object):
    def __init__(self):
        # JSON messages
        self.ser         = None
        self.json_active = False
        self.json_buffer = ''
        # Tuner state
        self.auto        = True
        self.bypass      = False
        self.power       = 0.0
        self.swr         = 0.0
        self.antenna     = None     # power at the antenna, when the tuner reports it
        self.efficiency  = None     # percent, likewise
        self.fwd_mv      = None     # detector voltages from request_raw()
        self.rev_mv      = None
        self.event       = ''       # last event
        self.tuning      = False    # between the Tune and Tuned events
        self.high_swr    = False    # HighSWR seen, cleared when the SWR is below the limit again
        self.info        = {}       # Board, Credit, FW, Build
        self.order       = 'LC'
        self.capacitance = 0
        self.inductance  = 0
        self.tune_ended  = False    # Tuned (or Reset) seen since the last Tune
        self.relay_order = False
        self.relay_cap   = [False, False, False, False, False, False, False]
        self.relay_ind   = [False, False, False, False, False, False, False]
        # Tuner settings
        self.eeprom      = []
        for address in range(0x100):
            self.eeprom.append(0xff)
        self.eeprom_known = set()   # cells read from the tuner (the rest are guesses)
        self.value_cap   = [0, 0, 0, 0, 0, 0, 0]
        self.value_ind   = [0, 0, 0, 0, 0, 0, 0]


    def connect(self, port):
        # Open the serial port

        try:
            self.ser = serial.serial_for_url(port,
                    baudrate = 4800,
                    bytesize=serial.EIGHTBITS,
                    stopbits = serial.STOPBITS_ONE,
                    parity = serial.PARITY_NONE,
                    timeout=0.1) # default timeout for reading in seconds
        # exit if the port is not opened
        except serial.SerialException as e:
            print(f'Serial port fail: {e}')
            sys.exit(10)

        logging.info('Succefully connected to ATU-100')


    def wait_name(self, name):
        timeout = 0
        while True:
            rxmsg = self.getmsg()
            if rxmsg and name in rxmsg:
                return rxmsg
            timeout += 1
            if timeout > 30:    # 3 Seconds
                logging.warning(f'Timeout waiting for {name}')
                return None


    def sendbool(self, name, value):
        if value:
            msg = f'{{"{name}": true}}\n'
        else:
            msg = f'{{"{name}": false}}\n'
        self.ser.write(msg.encode('ascii'))


    def sendint(self, name, value):
        msg = f'{{"{name}": {value}}}\n'
        self.ser.write(msg.encode('ascii'))


    def sendstr(self, name, value):
        msg = f'{{"{name}": "{value}"}}\n'
        self.ser.write(msg.encode('ascii'))


    def send_restart(self):
        self.sendbool('x', True)

    def geteepromdata(self, address):
        msg = f'{{"Get": {address}}}\n'
        self.ser.write(msg.encode('ascii'))


    def seteepromdata(self, address, data):
        msg = f'{{"Set": {address}, "Value": {data}}}\n'
        self.ser.write(msg.encode('ascii'))


    def send_relay_capacitors(self):
        bit = 0x1
        relaybits = 0
        for relay in range(7):
            if self.relay_cap[relay]:
                relaybits += bit
            bit *= 2
        if self.relay_order:
            relaybits += 128
        self.sendint('RelayC', relaybits)
        self.sendbool('Status', True)


    def send_relay_inductors(self):
        bit = 0x1
        relaybits = 0
        for relay in range(7):
            if self.relay_ind[relay]:
                relaybits += bit
            bit *= 2
        self.sendint('RelayI', relaybits)
        self.sendbool('Status', True)


    def get_relay_capacitors(self):
        self.geteepromdata(cell.EEPROM_LAST_SW)
        self.wait_name('fd')
        self.geteepromdata(cell.EEPROM_LAST_CAP)
        self.wait_name('ff')


    def get_relay_inductors(self):
        self.geteepromdata(cell.EEPROM_LAST_IND)
        self.wait_name('fe')


    def get_bcd(self, bcd):
        tens = bcd // 16
        units = bcd & 0xf
        return tens * 10 + units


    def getmsg(self):
        result = None
        rxd = self.ser.read()
        if not rxd:
            return None

        if self.json_active:
            while (rxd):
                ch = rxd.decode('ascii', errors='replace')
                logging.debug(f'Got {ch}')
                if ch == '{':  # a new message: the last one lost its end
                    logging.warning(f'Damaged JSON dropped: [{self.json_buffer}]')
                    self.json_buffer = ''
                elif ch in '\r\n':  # the tuner's messages are one line each
                    logging.warning(f'Damaged JSON dropped: [{self.json_buffer}]')
                    self.json_active = False
                    return None
                self.json_buffer += ch
                if ch == '}':
                    logging.debug('JSON ended')
                    self.json_active = False
                    try:
                        result = json.loads( self.json_buffer)
                    except:
                        logging.warning(f'Invalid JSON reveived: [{self.json_buffer}]')
                    if isinstance(result, dict) and 'ESP32' in result:
                        result = None  # the ESP32 bridge's own messages, not the tuner's

                    if result:

                        # We have a valid JSON message from the tuner, check the info in it

                        self.update(result)

                        for name in result:
                            value = result[name]
                            if all(char in string.hexdigits for char in name):
                                cell_address = int(name, 16)
                                cell_data = int(value, 16)
                                if (cell_address < 0x100) and (cell_data < 0x100):

                                    # We have a EEPROM cell value, save it and use to get setting info

                                    self.eeprom[cell_address] = cell_data
                                    self.eeprom_known.add(cell_address)
                                    if (cell_address >= cell.EEPROM_INDUCTOR_FIRST) and (cell_address <= cell.EEPROM_INDUCTOR_LAST):
                                        offset = cell_address - cell.EEPROM_INDUCTOR_FIRST
                                        highbyte = (offset % 2) == 0
                                        offset //= 2
                                        if highbyte:
                                            self.value_ind[offset] = self.get_bcd(cell_data) * 100
                                        else:
                                            self.value_ind[offset] += self.get_bcd(cell_data)
                                            logging.info(f'Inductor {offset} is {self.value_ind[offset]}')

                                    elif (cell_address >= cell.EEPROM_CAPACITOR_FIRST) and (cell_address <= cell.EEPROM_CAPACITOR_LAST):
                                        offset = cell_address - cell.EEPROM_CAPACITOR_FIRST
                                        highbyte = (offset % 2) == 0
                                        offset //= 2
                                        if highbyte:
                                            self.value_cap[offset] = self.get_bcd(cell_data) * 100
                                        else:
                                            self.value_cap[offset] += self.get_bcd(cell_data)
                                            logging.info(f'Capacitor {offset} is {self.value_cap[offset]}')

                                    elif cell_address == cell.EEPROM_LAST_SW:
                                        self.relay_order = cell_data == 1
                                            
                                    elif cell_address == cell.EEPROM_LAST_IND:
                                        bit = 0x1
                                        for relay in range(7):
                                            self.relay_ind[relay] = bool(bit & cell_data)
                                            bit *= 2

                                    elif cell_address == cell.EEPROM_LAST_CAP:
                                        bit = 0x1
                                        for relay in range(7):
                                            self.relay_cap[relay] = bool(bit & cell_data)
                                            bit *= 2

                    return result

                rxd = self.ser.read()
        else:
            while (rxd):
                ch = rxd.decode('ascii', errors='replace')
                logging.debug(f'Wait {ch}')
                if ch == '{':
                    logging.debug('JSON started')
                    self.json_active = True
                    self.json_buffer = ch
                    return None
                rxd = self.ser.read()

        return None


    def toggle_auto(self):
        self.auto = not self.auto
        logging.info(f'Auto -> {self.auto}')
        self.sendbool('Auto', self.auto)


    def toggle_bypass(self):
        self.bypass = not self.bypass
        logging.info(f'Bypass -> {self.bypass}')
        self.sendbool('Bypass', self.bypass)


    def start_tune(self):
        logging.info('Requesting tune')
        self.sendbool('Tune', True)


    def request_raw(self):
        # Detector voltages, answered with {"Fwd": mV, "Rev": mV}
        self.sendbool('Raw', True)


    def set_swr_alarm(self, swr):
        # HighSWR event limit, 1.1 to 9.9, or 0 for off.  Applies at once.
        tenths = round(swr * 10)
        if tenths != 0 and not 11 <= tenths <= 99:
            raise ValueError('SWR alarm must be 0 (off) or 1.1 to 9.9')
        self.seteepromdata(cell.EEPROM_SWR_ALARM, (tenths // 10) * 16 + tenths % 10)




    def update(self, rxmsg):
        # Keep the tuner state from any message it sends
        for name, value in rxmsg.items():
            if name == 'Auto':
                self.auto = value
            elif name == 'Bypass':
                self.bypass = value
            elif name == 'Power':
                self.power = value
                if 'Antenna' not in rxmsg and 'Order' in rxmsg:  # only sent while the tuner shows it (status lines)
                    self.antenna = None
                    self.efficiency = None
            elif name == 'Antenna':
                self.antenna = value
            elif name in ('Efficiency', 'efficency'):  # older builds spelt it so
                self.efficiency = value
            elif name == 'SWR':
                self.swr = value
                limit = self.swr_alarm()
                # Red while the SWR is at the limit. 0.00 is no power (a gap
                # between CW dits): keep the colour. Unknown limit: the
                # HighSWR event alone decides
                if limit and value > 0:
                    self.high_swr = value >= limit
            elif name == 'Order':
                self.order = value
                self.relay_order = value == 'LC'
            elif name == 'Capacitance':
                self.capacitance = value
                bits = self._relays_for(value, self.value_cap)
                if bits:
                    self.relay_cap = bits
            elif name == 'Inductance':
                self.inductance = value
                bits = self._relays_for(value, self.value_ind)
                if bits:
                    self.relay_ind = bits
            elif name == 'RelayI':  # the relays themselves (this firmware), exact
                self.relay_ind = [bool(value >> i & 1) for i in range(7)]
            elif name == 'RelayC':
                self.relay_cap = [bool(value >> i & 1) for i in range(7)]
                self.relay_order = bool(value & 0x80)
            elif name == 'Fwd':
                self.fwd_mv = value
            elif name == 'Rev':
                self.rev_mv = value
            elif name in ('Board', 'Credit', 'FW', 'Build'):
                self.info[name] = value
            elif name == 'Event':
                self.event = value
                if value == 'Tune':
                    self.tuning = True
                    self.tune_ended = False
                elif value in ('Tuned', 'Reset', 'Startup', 'Restart'):
                    self.tune_ended = True
                    self.tuning = False
                elif value == 'HighSWR':
                    self.high_swr = True


    def _relays_for(self, total, values):
        # Which relays make up a total. Each relay value is larger than all the
        # smaller ones together, so taking the largest that fits finds the only
        # answer. None when the values are not known yet or do not add up.
        if not all(values):
            return None
        bits = [False] * 7
        for relay in range(6, -1, -1):
            if values[relay] <= total:
                bits[relay] = True
                total -= values[relay]
        return bits if total == 0 else None


    def swr_alarm(self):
        # HighSWR limit from the EEPROM copy, None when not known or off
        if cell.EEPROM_SWR_ALARM not in self.eeprom_known:
            return None
        bcd = self.eeprom[cell.EEPROM_SWR_ALARM]
        if bcd == 0xff:
            return 3.0
        if bcd == 0:
            return None
        return self.get_bcd(bcd) / 10


    def _print_msg(self, rxmsg):
        for name, value in rxmsg.items():
            if name in ('Auto', 'Bypass'):
                print(f'{name + ":":12} {"Enabled" if value else "Disabled"}')

            elif name in ('Power', 'Antenna'):
                print(f'{name + ":":12} {value:.1f} W')

            elif name in ('Efficiency', 'efficency'):
                print(f'Efficiency:  {value} %')

            elif name == 'SWR':
                print(f'SWR:         {value:.2f}')

            elif name == 'Capacitance':
                print(f'Capacitance: {value} pF')

            elif name == 'Inductance':
                print(f'Inductance:  {value} nH')

            elif name in ('Fwd', 'Rev'):
                print(f'{"Forward:" if name == "Fwd" else "Reflected:":12} {value} mV')

            else:
                print(f'{name + ":":12} {value}')


    def main(self):
        # Get the command line args

        epilog_str = ("Commands can be:\n"
                      " 'a' - Disable auto tuning\n"
                      " 'A' - Enable auto tuning\n"
                      " 'b' - Disable bypass\n"
                      " 'B' - Enable bypass\n"
                      " 'c' - Capacitor relay off\n"
                      " 'C' - Capacitor relay on\n"
                      " 'd' - Download EEPROM contents from ATU-100\n"
                      " 'f' - Capacitor first\n"
                      " 'g' - Get an EEPROM cell\n"
                      " 'i' - Inductor relay off\n"
                      " 'I' - Inductor relay on\n"
                      " 'l' - Capacitor last\n"
                      " 'm' - Monitor: show everything the tuner sends until Ctrl-C\n"
                      " 'r' - Reset tuner, C and L\n"
                      " 's' - Set an EEPROM cell\n"
                      " 't' - Tune (transmit a carrier between the Tune and Tuned events)\n"
                      " 'u' - Upload EEPROM contents to ATU-100\n"
                      " 'v' - Detector voltages\n"
                      " 'x' - Restart the tuner\n"
                      "If no command supplied will show current status\n"
                      "Address and data are assumed to be hex format\n"
                      "The port can also be a URL such as socket://atu100.local:7000\n"
                      "and defaults to $ATU100_PORT if set\n"
        )
        parser = argparse.ArgumentParser(prog='atu-100',
                                         description='Control program for ATU-100 tuner',
                                         epilog=epilog_str,
                                         formatter_class=argparse.RawTextHelpFormatter)

        parse_general = parser.add_argument_group('General', 'General options')
        parse_general.add_argument('-p', '--port', default=os.environ.get('ATU100_PORT', '/dev/ttyACM0'), help = 'Serial port or URL for ATU-100')
        parse_general.add_argument('-c', '--command', default='', help = 'Command, as listed below')
        parse_general.add_argument('--logfile', default='atu100.log', help = 'Log filename (atu100.log)')
        parse_general.add_argument('--log-level', type=str,
                                    dest='log_level', default='info',
                                    choices=['debug', 'info', 'warn', 'error', 'critical'],
                                    help='Log level: debug|info(default)|warn|error|critical')
        parse_general.add_argument('--newlog', default=False, action='store_true', help = 'Create a new log file')
        parse_config = parser.add_argument_group('Config', 'EEPROM config options')
        parse_config.add_argument('-e', '--eepromfile', default='', help = 'EEPROM file anme for EEPROM dump or upload')
        parse_config.add_argument('-a', '--address', default='', help = 'EEPROM address for get or set commands')
        parse_config.add_argument('-d', '--data', default='', help = 'EEPROM data for set command')
        parse_config.add_argument('-r', '--relay', default=0, help = 'Relay number, 0 to 6')
        def swr_limit(text):
            value = float(text)
            if value != 0 and not 1.1 <= value <= 9.9:
                raise argparse.ArgumentTypeError('1.1 to 9.9, or 0 for off')
            return value
        parse_config.add_argument('--swr-alarm', type=swr_limit, help = 'Set the HighSWR event limit, 1.1 to 9.9, e.g. 2.5, or 0 for off')

        self.args = parser.parse_args()

        # Set up logging

        FORMAT = '%(asctime)-15s %(levelname)s %(message)s'
        if self.args.newlog:
            filemode = 'w'
        else:
            filemode = 'a'
        logging.basicConfig(level=logging.getLevelName(self.args.log_level.upper()), format=FORMAT, filename=self.args.logfile, filemode=filemode)
        logging.info('=======================')
        logging.info('| Starting atu100.py |')
        logging.info('=======================')

        self.connect(self.args.port)

        # Get information from the tuner

        if self.args.swr_alarm is not None:
            print(f'HighSWR limit: {self.args.swr_alarm:.1f}' if self.args.swr_alarm else 'HighSWR event off')
            self.set_swr_alarm(self.args.swr_alarm)
            self.wait_name(f'{cell.EEPROM_SWR_ALARM:02x}')
            return

        if self.args.command == 'a':
            print('Disabling auto tuner mode')
            self.sendbool('Auto', False)

        elif self.args.command == 'A':
            print('Enabling auto tuner mode')
            self.sendbool('Auto', True)

        elif self.args.command == 'b':
            print('Disabling bypass')
            self.sendbool('Bypass', False)

        elif self.args.command == 'B':
            print('Enabling bypass')
            self.sendbool('Bypass', True)

        elif self.args.command == 'd':
            print('Downloading EEPROM, this will take about 10 seconds')
            self.sendstr('Dump', 'EEPROM')

        elif self.args.command == 'u':
            if self.args.eepromfile == '':
                print('Use the -e to specifiy the EEPROM JSON file to upload')
                sys.exit(20)
            try:
                with open(self.args.eepromfile, 'r') as jsonfile:
                    jsondata = jsonfile.read()
                    upload = json.loads(jsondata)
            except Exception as e:
                print(f'EEPROM JSON file read error: {e}')
                sys.exit(22)
            print('Checking EEPROM, this will take about 10 seconds')
            self.sendstr('Dump', 'EEPROM')
            rxmsg = self.wait_name('ff')
            if not 'ff' in rxmsg:
                print('Failed to check existing EEPROM contents')
                sys.exit(25)
            print('Uploading changes')
            for address in upload:
                eepromaddress = int(address, 16)
                if self.eeprom[eepromaddress] != int(upload[address], 16):
                    print(f'  Cell {address}: {self.eeprom[eepromaddress]:02x} replaced with {upload[address]}')
                    self.seteepromdata(eepromaddress, int(upload[address], 16))
                    self.wait_name(address)
            print('Upload complete')
            return
            
        elif self.args.command == 'f':
            print('Turn on capacitor first relay i.e CL')
            self.get_relay_capacitors()
            self.relay_order = False
            self.send_relay_capacitors()

        elif self.args.command == 'l':
            print('Turn off capacitor first relay i.e LC')
            self.get_relay_capacitors()
            self.relay_order = True
            self.send_relay_capacitors()

        elif self.args.command == 'c':
            print(f'Turn off capacitor relay {self.args.relay}')
            self.get_relay_capacitors()
            self.relay_cap[int(self.args.relay)] = False
            self.send_relay_capacitors()

        elif self.args.command == 'C':
            print(f'Turn on capacitor relay {self.args.relay}')
            self.get_relay_capacitors()
            self.relay_cap[int(self.args.relay)] = True
            self.send_relay_capacitors()

        elif self.args.command == 'i':
            print(f'Turn off inductor relay {self.args.relay}')
            self.get_relay_inductors()
            self.relay_ind[int(self.args.relay)] = False
            self.send_relay_inductors()

        elif self.args.command == 'I':
            print(f'Turn on inductor relay {self.args.relay}')
            self.get_relay_inductors()
            self.relay_ind[int(self.args.relay)] = True
            self.send_relay_inductors()

        elif self.args.command == 'r':
            print('Resetting settings')
            self.sendbool('Reset', True)

        elif self.args.command == 't':
            print('Starting tuning: transmit a carrier until the Tuned event')
            self.sendbool('Tune', True)

        elif self.args.command == 'v':
            self.request_raw()

        elif self.args.command == 'x':
            print('Restarting the tuner')
            self.send_restart()

        elif self.args.command == 'm':
            print('Showing everything the tuner sends, Ctrl-C to stop')
            self.sendbool('Status', True)

        elif self.args.command == 'g':
            if self.args.address == '':
                print('Use -a option to set address to get from')
                sys.exit(30)
            self.geteepromdata(int(self.args.address, 16))

        elif self.args.command == 's':
            if self.args.address == '':
                print('Use -a option to set address to set data of')
                sys.exit(40)
            if self.args.data == '':
                print('Use -d option to set data value to be set')
                sys.exit(50)
            self.seteepromdata(int(self.args.address, 16), int(self.args.data, 16))

        else:
            self.sendbool('Status', True)

        #
        # Loop processing waiting for last message
        #

        while True:

            # Check for a new JSON message

            try:
                rxmsg = self.getmsg()
            except KeyboardInterrupt:
                break
            if rxmsg:
                logging.debug(rxmsg)

                # Monitor: everything, one line each

                if self.args.command == 'm':
                    print(datetime.now().strftime('%H:%M:%S'), json.dumps(rxmsg))
                    continue

                print()

                # Events: show them, and keep waiting for the status after a tune

                if 'Event' in rxmsg:
                    self._print_msg(rxmsg)
                    if self.args.command == 'x' and rxmsg['Event'] in ('Startup', 'Restart'):
                        break
                    continue

                # Detector voltages

                if self.args.command == 'v':
                    if 'Fwd' in rxmsg:
                        self._print_msg(rxmsg)
                        break
                    continue

                # EEPROM Dump command

                # A status line may come first while there is power: wait
                # for the answer itself
                if self.args.command == 'd' and 'ff' not in rxmsg:
                    continue
                if self.args.command in ('g', 's') and f'{int(self.args.address, 16):02x}' not in rxmsg:
                    continue

                if self.args.command == 'd':
                    print(json.dumps(rxmsg, indent=4))
                    if self.args.eepromfile != '':
                        with open(self.args.eepromfile, 'w') as jsonfile:
                            jsonfile.write(json.dumps(rxmsg, indent=4))
                        print(f'Saved to {self.args.eepromfile}')
                    break

                # Get EEPROM cell
                
                elif self.args.command == 'g':
                    for name in rxmsg:
                        print(f'{name} = {rxmsg[name]}')
                    break

                # Set EEPROM cell

                elif self.args.command == 's':
                    for name in rxmsg:
                        print(f'{name} = {rxmsg[name]}')
                    break

                else:
                    result = 'Power' in rxmsg
                    if self.args.command == 't' and not self.tune_ended:
                        result = False  # a status from before the tune ended
                    self._print_msg(rxmsg)
                    if result:
                        break

        # Clean up and exit

        logging.info('Exiting')




if __name__ == "__main__":
    atu = atu100()
    atu.main()
    sys.exit(0)