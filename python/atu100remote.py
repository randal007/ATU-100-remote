#!/usr/bin/env python3
'''
ATU-100 control program

This program provides a ncurse command line for routine control of a remote
ATU-100.

The ATU-100 needs the firmware from
https://github.com/randal007/ATU-100-remote (branch display-buttons-uart);
see README.md there for wiring and the messages.  The port can be a serial
device or a pyserial URL such as socket://atu100.local:7000.
'''

import argparse
import curses
import datetime
import json
import logging
import os
import serial
import sys
import atu100

from curses import wrapper
from datetime import datetime
from enum import Enum
from time import sleep, time



# ncurses related

# Colours

C_GOOD_DATA  = 1
C_BAD_DATA   = 2
C_WARN_DATA  = 3
C_NOTE_DATA  = 4
C_COMPLETE   = 5
C_EMPTY_DATA = 6

# Screen locations

SCREEN_WIDTH  = 48
SCREEN_HEIGHT =  8

SDATA_FIRST        = 10
SDATA_FIRST_WIDTH  =  8
SDATA_SECOND       = 35
SDATA_SECOND_WIDTH =  8

POWER_R     = 1
POWER_C     = SDATA_FIRST

SWR_R       = 2
SWR_C       = SDATA_FIRST

ANTENNA_R   = 3
ANTENNA_C   = SDATA_FIRST

EFF_R       = 4
EFF_C       = SDATA_FIRST + 4

AUTO_R      = 5
AUTO_C      = SDATA_FIRST

EVENT_R     = 6
EVENT_C     = SDATA_FIRST

ORDER_R     = 1
ORDER_C     = SDATA_SECOND

CAP_R       = 2
CAP_C       = SDATA_SECOND

IND_R       = 3
IND_C       = SDATA_SECOND

DETECTOR_R  = 4
DETECTOR_C  = SDATA_SECOND - 3   # "2697/1263", 11 wide

BYPASS_R    = 5
BYPASS_C    = SDATA_SECOND

TUNE_R      = 6
TUNE_C      = SDATA_SECOND

RAW_POLL    = 1.0   # seconds between detector readings while 'v' is on


class atu100remote(object):
    def __init__(self):
        self.atu = atu100.atu100()
        self.raw_on = False
        self.raw_last = 0

        # # JSON messages
        # self.ser         = None
        # self.json_active = False
        # self.json_buffer = ''
        # # Tuner state
        # self.auto        = True
        # self.bypass      = False
        # self.power       = 0.0
        # self.swr         = 0.0
        # self.reverse     = 0.0
        # self.order       = 'LC'
        # self.capacitance = 0
        # self.inductance  = 0

        # Get the command line args

        epilog_str = ("Keys to control program:\n"
                      " 'q' or ESC - exit\n"
                      " 'a'        - Toggle auto tuning\n"
                      " 'b'        - Toggle bypass\n"
                      " 'r'        - Reset tuner, C and L\n"
                      " 's'        - Ask for the status\n"
                      " 't'        - Tune (transmit a carrier until Tune shows Done)\n"
                      " 'v'        - Detector voltages once a second, on/off\n"
        )
        parser = argparse.ArgumentParser(prog='atu-100 remote',
                                         description='Control program for ATU-100 tuner',
                                         epilog=epilog_str,
                                         formatter_class=argparse.RawTextHelpFormatter)

        parse_general = parser.add_argument_group('General', 'General options')
        parse_general.add_argument('-p', '--port', default=os.environ.get('ATU100_PORT', '/dev/ttyACM0'), help = 'Serial port or URL for ATU-100')
        parse_general.add_argument('--logfile', default='atu100.log', help = 'Log filename (atu100.log)')
        parse_general.add_argument('--log-level', type=str,
                                    dest='log_level', default='info',
                                    choices=['debug', 'info', 'warn', 'error', 'critical'],
                                    help='Log level: debug|info(default)|warn|error|critical')
        parse_general.add_argument('--newlog', default=False, action='store_true', help = 'Create a new log file')

        self.args = parser.parse_args()

        # Set up logging

        FORMAT = '%(asctime)-15s %(levelname)s %(message)s'
        if self.args.newlog:
            filemode = 'w'
        else:
            filemode = 'a'
        logging.basicConfig(level=logging.getLevelName(self.args.log_level.upper()), format=FORMAT, filename=self.args.logfile, filemode=filemode)
        logging.info('============================')
        logging.info('| Starting atu100remote.py |')
        logging.info('============================')

        # Open the serial port

        try:
            self.atu.connect(self.args.port)
        # exit if the port is not opened
        except serial.SerialException as e:
            sys.exit(e)

        # Load the JSON files

        # try:
        #     with open(self.args.scpidev) as json_file:
        #         self.scpi_devices = json.load(json_file)
        # except:
        #     logging.error(f"Can't read {self.args.scpidev} SCPI devices file")
        #     print(f"Can't read {self.args.scpidev} SCPI devices file.  Use find_devices.py to create it if missing")
        #     exit(10)





    def initlayout(self):
        # Set up the ncurses style window

        curses.curs_set(False)
        if curses.can_change_color():  # many terminals (tmux, ssh, console) cannot
            try:
                curses.init_color(curses.COLOR_BLACK, 0, 0, 0)
            except curses.error:
                pass
        curses.init_pair(C_GOOD_DATA,  curses.COLOR_GREEN,   curses.COLOR_BLACK)
        curses.init_pair(C_BAD_DATA,   curses.COLOR_WHITE,   curses.COLOR_RED)
        curses.init_pair(C_WARN_DATA,  curses.COLOR_MAGENTA, curses.COLOR_BLACK)
        curses.init_pair(C_NOTE_DATA,  curses.COLOR_BLUE,    curses.COLOR_BLACK)
        curses.init_pair(C_COMPLETE,   curses.COLOR_WHITE,   curses.COLOR_GREEN)
        curses.init_pair(C_EMPTY_DATA, curses.COLOR_WHITE,   curses.COLOR_BLACK)
        self.mwin.clear()
        self.mwin.nodelay(True)

        #  Create windows         Lines, Columns, Down, Across
#        self.swin = curses.newwin(curses.LINES - 1, curses.COLS - 1, 0, 0)
        self.swin = curses.newwin(SCREEN_HEIGHT, SCREEN_WIDTH, 0, 0)

        self.swin.border()
        self.swin.addstr(0, 2, ' ATU-100 Remote ')

        # Render the static status text
        self.swin.addstr(1, 1, 'Power:      ---.- W  Order:             --')
        self.swin.addstr(2, 1, 'SWR:         -.--    Capacitance:    ----- pF')
        self.swin.addstr(3, 1, 'Antenna:    ---.- W  Inductance:     ----- nH')
        self.swin.addstr(4, 1, 'Efficiency:   --- %  Detector:     ---/--- mV')
        self.swin.addstr(5, 1, 'Auto:    --------    Bypass:      -------- ')
        self.swin.addstr(6, 1, 'Event:               Tune:')

        self.mwin.noutrefresh()
        self.swin.noutrefresh()
        curses.doupdate()



    def update_var(self, var, value, attr = C_GOOD_DATA):
        width = SDATA_FIRST_WIDTH
        rjust = True
        if var == 'Power':
            r = POWER_R
            c = POWER_C

        elif var == 'SWR':
            r = SWR_R
            c = SWR_C

        elif var == 'Antenna':
            r = ANTENNA_R
            c = ANTENNA_C

        elif var == 'Efficiency':
            r = EFF_R
            c = EFF_C
            width = 4

        elif var == 'Detector':
            r = DETECTOR_R
            c = DETECTOR_C
            width = 11

        elif var == 'Auto':
            r = AUTO_R
            c = AUTO_C

        elif var == 'Event':
            r = EVENT_R
            c = EVENT_C

        elif var == 'Order':
            r = ORDER_R
            c = ORDER_C

        elif var == 'Capacitance':
            r = CAP_R
            c = CAP_C

        elif var == 'Inductance':
            r = IND_R
            c = IND_C

        elif var == 'Bypass':
            r = BYPASS_R
            c = BYPASS_C

        elif var == 'Tune':
            r = TUNE_R
            c = TUNE_C

        else:
            logging.warning(f'Unknown var {var} with value {value}')
            return

        try:
            if rjust:
                self.swin.addnstr(r, c, value.rjust(width), width, curses.color_pair(attr))
            else:
                self.swin.addnstr(r, c, value.ljust(width), width, curses.color_pair(attr))
        except:
            pass
        self.swin.noutrefresh()

    def toggle_auto(self):
        self.update_var('Auto', '--------', C_EMPTY_DATA)
        self.atu.toggle_auto()


    def toggle_bypass(self):
        self.update_var('Bypass', '--------', C_EMPTY_DATA)
        self.atu.toggle_bypass()


    def start_tune(self):
        self.update_var('Tune', 'Sent')
        self.atu.start_tune()


    def show_state(self, rxmsg):
        # Show what changed with this message; the atu100 object keeps the state
        atu = self.atu
        if 'Auto' in rxmsg:
            self.update_var('Auto', 'Enabled' if atu.auto else 'Disabled')
        if 'Bypass' in rxmsg:
            self.update_var('Bypass', 'Enabled' if atu.bypass else 'Disabled')
        if 'Power' in rxmsg:
            self.update_var('Power', f'{atu.power:.1f}')
            self.update_var('Antenna', '---.-' if atu.antenna is None else f'{atu.antenna:.1f}', C_GOOD_DATA if atu.antenna is not None else C_EMPTY_DATA)
            self.update_var('Efficiency', '---' if atu.efficiency is None else f'{atu.efficiency}', C_GOOD_DATA if atu.efficiency is not None else C_EMPTY_DATA)
        if 'SWR' in rxmsg:
            self.update_var('SWR', f'{atu.swr:.2f}', C_BAD_DATA if atu.high_swr else C_GOOD_DATA)
        if 'Order' in rxmsg:
            self.update_var('Order', atu.order)
        if 'Capacitance' in rxmsg:
            self.update_var('Capacitance', f'{atu.capacitance}')
        if 'Inductance' in rxmsg:
            self.update_var('Inductance', f'{atu.inductance}')
        if 'Fwd' in rxmsg:
            self.update_var('Detector', f'{atu.fwd_mv}/{atu.rev_mv}')
        if 'Event' in rxmsg:
            event = rxmsg['Event']
            self.update_var('Event', event, C_BAD_DATA if event in ('HighSWR', 'Overload', 'Error') else C_GOOD_DATA)
            if event == 'Tune':
                self.update_var('Tune', 'Tuning', C_WARN_DATA)
            elif event == 'Tuned':
                self.update_var('Tune', 'Done', C_COMPLETE)
            elif event == 'Reset':
                self.update_var('Tune', ' ')
            elif event == 'HighSWR':
                self.update_var('SWR', f'{atu.swr:.2f}', C_BAD_DATA)
        if 'Build' in rxmsg:
            self.swin.addstr(0, 2, f' ATU-100 Remote, {atu.info.get("Build", "")} ')


    def main(self, stdscr):
        self.mwin = stdscr
        self.initlayout()

        # Enable mouse events

        curses.mousemask(curses.ALL_MOUSE_EVENTS)
        print('\033[?1003h')

        # Get inital information from the tuner

        self.atu.sendbool('Status', True)

        #
        # Loop processing key presses, send polls and update battery voltage etc
        #

        while True:

            # Process key presses

            key = self.mwin.getch()
            if key == ord('q') or key == 27:   # 'q' or ESC - Quit program
                # Stop mouse events
                print('\033[?1003l')
                break

            elif key == curses.KEY_MOUSE:
                try:
                    event = curses.getmouse()
                    if event[4] == curses.BUTTON1_DOUBLE_CLICKED:
                        if event[1] < (SDATA_FIRST + SDATA_FIRST_WIDTH):
                            if event[2] == AUTO_R:
                                self.toggle_auto()
                        else:
                            if event[2] == BYPASS_R:
                                self.toggle_bypass()
                            elif event[2] == TUNE_R:
                                self.start_tune()
                except:
                    pass                                

            elif key == ord('a'):
                self.toggle_auto()

            elif key == ord('b'):
                self.toggle_bypass()

            elif key == ord('r'):
                logging.info('Requesting reset')
                self.atu.sendbool('Reset', True)

            elif key == ord('s'):
                logging.info('Requesting status')
                self.update_var('Auto', '--------', C_EMPTY_DATA)
                self.update_var('Bypass', '--------', C_EMPTY_DATA)
                self.update_var('Event', ' ') 
                self.atu.sendbool('Status', True)

            elif key == ord('t'):
                self.start_tune()

            elif key == ord('v'):
                self.raw_on = not self.raw_on
                if not self.raw_on:
                    self.update_var('Detector', '---/---', C_EMPTY_DATA)

            # Detector voltages once a second while 'v' is on

            if self.raw_on and time() - self.raw_last >= RAW_POLL and not self.atu.tuning:
                self.raw_last = time()
                self.atu.request_raw()

            elif key != -1:                    # Not 'no key' - Show key usage
                 logging.warning(f'Unknown key: {key}')

            # Check for a new JSON message

            rxmsg = self.atu.getmsg()
            if rxmsg:
                logging.debug(rxmsg)
                self.show_state(rxmsg)

            self.mwin.noutrefresh()
            curses.doupdate()


        # Clean up and exit

        logging.info('Exiting')




if __name__ == "__main__":
    atu = atu100remote()
    wrapper(atu.main)
    exit(0)