/*  Runs the real firmware on a PC with simulated time.

    Every 208 instruction cycles (the Timer2 period) it plays one tick of:
      - the serial partner, an Arduino Mega at 16 MHz (4807.7 baud),
        optionally off by BAUD_ERR percent, with random bit phase
      - the buttons, the RF on the input and an antenna behind the
        relay network, read back through the ADC
      - the interrupt, and checks: watchdog timeout, LED writes on the
        display's I2C pins, framing errors on the Arduino side.
    A scenario drives the tuner over serial and the buttons and checks
    what comes back. Exit status is the number of failed checks. */

#define _GNU_SOURCE
#include <complex.h>
#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <termios.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>
#include "xc.h"

volatile porta_t PORTA_r;
volatile portb_t PORTB_r;
volatile portc_t PORTC_r;
volatile lata_t LATA_r;
volatile latb_t LATB_r;
volatile latc_t LATC_r;
volatile trisa_t TRISA_r;
volatile trisb_t TRISB_r;
volatile trisc_t TRISC_r;
struct misc_bits misc_r;
uint8_t OSCCON, FVRCON, ADCON1, ADRESL, ADRESH, PR2, T2CON, TXREG;

void fw_main(void);
void isr(void);
void host_show_display(const char *title);
const char *host_display_row(int r);
extern unsigned char initial_eeprom[256];

#define CYCLES_PER_SEC 4000000.0
#define TICK 208

static unsigned long long now, next_tick = TICK, last_clrwdt;
static int failures, display_type;

static double secs(void) { return now / CYCLES_PER_SEC; }

static void fail(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    printf("  FAIL at %.2f s: ", secs());
    vprintf(fmt, ap);
    printf("\n");
    va_end(ap);
    failures++;
}

static void check_display(int ok, const char *what);

static void check(int ok, const char *what)
{
    if (ok)
        printf("  ok   %s\n", what);
    else
        fail("%s", what);
}

// Display checks only apply when a display is fitted
static void check_display(int ok, const char *what)
{
    if (display_type != 0)
        check(ok, what);
}

/* ---------------- EEPROM ---------------- */

static unsigned char eeprom[256];
unsigned char eeprom_read(unsigned char addr) { return eeprom[addr]; }
void eeprom_write(unsigned char addr, unsigned char value) { eeprom[addr] = value; }

void host_reset(void)
{
    fail("firmware asked for a reset");
    exit(failures);
}

/* ---------------- RF and antenna ---------------- */

static double fwd_mv;              // forward detector voltage, 0 = no carrier
static double complex z_load = 35 + 30 * I; // antenna impedance, ohms
static double freq = 7.1e6;

static const int ind_nh[7] = {50, 110, 220, 450, 1000, 2200, 4500};
static const int cap_pf[7] = {10, 22, 47, 100, 220, 470, 1000};

static int relays_l(void)
{
    return LATB_r.LATB3 | LATA_r.LATA2 << 1 | LATB_r.LATB4 << 2 | LATA_r.LATA3 << 3 |
           LATB_r.LATB5 << 4 | LATA_r.LATA5 << 5 | LATA_r.LATA4 << 6;
}

static int relays_c(void)
{
    return LATC_r.LATC7 | LATC_r.LATC3 << 1 | LATC_r.LATC6 << 2 | LATC_r.LATC2 << 3 |
           LATC_r.LATC5 << 4 | LATC_r.LATC1 << 5 | LATC_r.LATC4 << 6;
}

static double swr_for(int lbits, int cbits, int c_out)
{
    double l = 0, c = 0, w = 2 * M_PI * freq;
    for (int i = 0; i < 7; i++) {
        if (lbits >> i & 1) l += ind_nh[i] * 1e-9;
        if (cbits >> i & 1) c += cap_pf[i] * 1e-12;
    }
    double complex zl = I * w * l, zin;
    double complex yc = I * w * c;
    if (c_out) // C on the antenna side
        zin = zl + 1 / (1 / z_load + yc);
    else
        zin = 1 / (1 / (zl + z_load) + yc);
    double g = cabs((zin - 50) / (zin + 50));
    return (1 + g) / (1 - g);
}

static double swr_now(void)
{
    return swr_for(relays_l(), relays_c(), LATC_r.LATC0);
}

// Lowest SWR any relay combination reaches
static double swr_best(void)
{
    double best = 1e9;
    for (int sw = 0; sw < 2; sw++)
        for (int l = 0; l < 128; l++)
            for (int c = 0; c < 128; c++) {
                double s = swr_for(l, c, sw);
                if (s < best) best = s;
            }
    return best;
}

static long adc_reads;

struct misc_bits *adc_hook(void)
{
    static const double ref[4] = {1024, 1024, 2048, 4096};
    if (misc_r.ADGO) { // conversion started by the previous statement
        adc_reads++;
        double mv = fwd_mv;
        if (misc_r.CHS == 0 && fwd_mv > 0) { // reverse detector
            double s = swr_now();
            mv = fwd_mv * (s - 1) / (s + 1);
        }
        if (getenv("TRACE_ADC") && misc_r.CHS == 0 && fwd_mv > 0)
            printf("    adc %.2f s L=%3d C=%3d sw=%d swr=%.2f\n", secs(), relays_l(), relays_c(), LATC_r.LATC0,
                   swr_now() > 99 ? 99 : swr_now());
        if (getenv("NOISE")) { // percent, roughly gaussian
            double g = 0;
            for (int i = 0; i < 6; i++)
                g += rand() / (double)RAND_MAX - 0.5;
            mv *= 1 + atof(getenv("NOISE")) / 100 * g * 1.4;
        }
        int counts = (int)(mv * 1023 / ref[FVRCON & 3]);
        if (counts > 1023) counts = 1023;
        ADRESL = counts & 0xff;
        ADRESH = counts >> 8;
        misc_r.ADGO = 0;
    }
    misc_r.FVRRDY = 1;
    return &misc_r;
}

/* ---------------- Buttons ---------------- */

static double button_until[3]; // RB0 tune, RB1 auto, RB2 bypass

static void press(int button, double seconds)
{
    button_until[button] = secs() + seconds;
}

/* ---------------- Serial partner ---------------- */

static double bit_cycles; // the Arduino's bit time

static char to_pic[8192];
static int to_pic_in, to_pic_out;
static int tx_active, tx_byte;
static double tx_start;

static void send(const char *s)
{
    while (*s)
        to_pic[to_pic_in++ % sizeof to_pic] = *s++;
}

static int arduino_tx_level(void)
{
    for (;;) {
        if (!tx_active) {
            if (to_pic_out == to_pic_in)
                return 1;
            tx_byte = (unsigned char)to_pic[to_pic_out++ % sizeof to_pic];
            tx_start = now - (rand() % TICK); // any phase against the PIC's ticks
            tx_active = 1;
        }
        int k = (int)((now - tx_start) / bit_cycles);
        if (k >= 10) { // done; a back to back byte starts right after the stop bit
            tx_active = 0;
            if (to_pic_out != to_pic_in) {
                tx_byte = (unsigned char)to_pic[to_pic_out++ % sizeof to_pic];
                tx_start += 10 * bit_cycles;
                tx_active = 1;
            }
            continue;
        }
        return k == 0 ? 0 : k <= 8 ? (tx_byte >> (k - 1)) & 1 : 1;
    }
}

static char from_pic[200000]; // everything the PIC sent
static int from_pic_len, mark, framing_errors;
static int rx_level = 1, rx_active, rx_bit, rx_byte;
static double rx_start;

static int pty = -1;      // PTY=1: the serial line is also a pseudo terminal
static int pty_side = -1; // our handle on the program's side of it

static void received(char c)
{
    static double line_start;
    if (pty >= 0) {
        if (write(pty, &c, 1) < 0) { /* nobody reading */ }
        if (getenv("QUIET"))
            return;
    }
    if (from_pic_len < (int)sizeof from_pic - 1) {
        if (from_pic_len == 0 || from_pic[from_pic_len - 1] == '\n')
            line_start = secs();
        from_pic[from_pic_len++] = c;
        from_pic[from_pic_len] = 0;
    }
    if (c == '\n') {
        char *line = from_pic + from_pic_len - 1;
        while (line > from_pic && line[-1] != '\n')
            line--;
        int n = (int)(from_pic + from_pic_len - line) - 1;
        if (n > 150)
            printf("  %7.2f  PIC> %.120s... (%d chars)\n", line_start, line, n);
        else
            printf("  %7.2f  PIC> %.*s\n", line_start, n, line);
    }
}

// Sample the PIC's output the way a UART does: from the start bit edge,
// in the middle of each of the Arduino's bit times.
static void arduino_rx(int new_level)
{
    while (rx_active && now >= rx_start + (rx_bit + 0.5) * bit_cycles) {
        if (rx_bit <= 8) {
            if (rx_bit >= 1)
                rx_byte |= rx_level << (rx_bit - 1);
        } else {
            if (!rx_level)
                framing_errors++;
            else
                received((char)rx_byte);
            rx_active = 0;
        }
        rx_bit++;
    }
    if (!rx_active && rx_level && !new_level) {
        rx_active = 1;
        rx_start = now;
        rx_bit = 1;
        rx_byte = 0;
    }
    rx_level = new_level;
}

// Display row contains s, ignoring the gaps between the 128*64 OLED's wide characters
static int display_has(int row, const char *s)
{
    char packed[40];
    int n = 0;
    for (const char *p = host_display_row(row); *p && n < 39; p++)
        if (*p != ' ')
            packed[n++] = *p;
    packed[n] = 0;
    return strstr(packed, s) != NULL;
}

// s is in a complete line received since the mark
static int got(const char *s)
{
    const char *end = strrchr(from_pic, '\n');
    return end && end > from_pic + mark &&
           memmem(from_pic + mark, (size_t)(end - from_pic - mark), s, strlen(s)) != NULL;
}

// A status message has arrived after the line containing s
static int got_status_after(const char *s)
{
    const char *p = strstr(from_pic + mark, s);
    return p && (p = strstr(p, "\"SWR\":")) != NULL && strchr(p, '\n') != NULL;
}

// Number after "name": in the last status message
static double last_value(const char *name)
{
    char key[40];
    snprintf(key, sizeof key, "\"%s\":", name);
    const char *p = NULL, *q = from_pic;
    while ((q = strstr(q, key)) != NULL)
        p = q++;
    return p ? atof(p + strlen(key)) : -1;
}

/* ---------------- Time ---------------- */

static void scenario(void);

// PTY mode: take what the program on the pty sends, and keep simulated
// time in step with the wall clock so its timeouts behave.
static void pty_tick(void)
{
    static struct timespec start;
    static unsigned long n;
    char buf[64];
    if (start.tv_sec == 0)
        clock_gettime(CLOCK_MONOTONIC, &start);
    static char line[200];
    static int len;
    ssize_t got_n = read(pty, buf, sizeof buf);
    for (ssize_t i = 0; i < got_n; i++) {
        to_pic[to_pic_in++ % sizeof to_pic] = buf[i];
        if (buf[i] == '\n' || len == sizeof line - 1) {
            line[len] = 0;
            printf("  %7.2f  PC>  %s\n", secs(), line);
            len = 0;
        } else if (buf[i] != '\r')
            line[len++] = buf[i];
    }
    if (getenv("RF_WAVE") && n % 200 == 0) { // speech-like carrier for the graphs
        double s = secs();
        fwd_mv = 900 * (0.35 + 0.65 * fabs(sin(s * 1.7)) * (0.6 + 0.4 * sin(s * 0.31)));
    }
    if (n % 20000 == 0) // every ~1 s: undo the exclusive lock a killed program left behind
        ioctl(pty_side, TIOCNXCL);
    if (++n % 200 == 0) { // every ~10 ms
        struct timespec t;
        clock_gettime(CLOCK_MONOTONIC, &t);
        double wall = (t.tv_sec - start.tv_sec) + (t.tv_nsec - start.tv_nsec) / 1e9;
        if (secs() > wall)
            usleep((useconds_t)((secs() - wall) * 1e6));
    }
}

static void tick(void)
{
    if (pty >= 0)
        pty_tick();
    else
        scenario();
    for (int b = 0; b < 3; b++) {
        int pressed = secs() < button_until[b];
        if (b == 0) PORTB_r.RB0 = !pressed;
        if (b == 1) PORTB_r.RB1 = !pressed;
        if (b == 2) PORTB_r.RB2 = !pressed;
    }
    PORTA_r.RA6 = arduino_tx_level();
    if (misc_r.TMR2IE && misc_r.GIE && PR2 == TICK - 1) {
        misc_r.TMR2IF = 1;
        isr();
    }
    arduino_rx(LATA_r.LATA7);
    if (now - last_clrwdt > CYCLES_PER_SEC)
        fail("watchdog would reset: %.2f s since CLRWDT", (now - last_clrwdt) / CYCLES_PER_SEC), last_clrwdt = now;
    if (display_type != 0 && (LATB_r.LATB6 || LATB_r.LATB7))
        fail("LED latch set on the display's I2C pins"), LATB_r.LATB6 = LATB_r.LATB7 = 0;
}

void host_delay(unsigned long cycles)
{
    unsigned long long end = now + cycles;
    while (next_tick <= end) {
        now = next_tick;
        next_tick += TICK;
        tick();
    }
    now = end;
}

void host_clrwdt(void)
{
    last_clrwdt = now;
    host_delay(4);
}

/* ---------------- Scenario ---------------- */

static int step;
static double step_time;

static void next(void)
{
    step = (step == 12) ? 125 : step + 1;
    step_time = secs();
    mark = from_pic_len;
}

static int waited(double s) { return secs() - step_time >= s; }

static int timeout(double s, const char *what)
{
    if (!waited(s))
        return 0;
    fail("timed out waiting for %s", what);
    next();
    return 1;
}

static void finish(void)
{
    printf("\n  serial framing errors seen by the Arduino: %d\n", framing_errors);
    if (framing_errors)
        failures++;
    printf("  %s: %d failed\n\n", failures ? "FAILED" : "PASSED", failures);
    exit(failures);
}

void tune(void);

/*  SWEEP=1: tune about a thousand loads across HF and compare with the
    best match the relays can reach. One line per load in sweep.csv.
    TUNE_MODE=0 selects N7DDC's original search, NOISE=n adds n% noise
    to the detector readings. */
static void sweep(void)
{
    static const double mhz[] = {1.85, 3.6, 5.35, 7.1, 10.1, 14.2, 18.1, 21.2, 24.9, 28.5};
    static const double r[] = {6, 10, 16, 25, 35, 50, 70, 100, 160, 250, 400, 600};
    static const double x[] = {-400, -200, -100, -50, -20, 0, 20, 50, 100, 200, 400};
    FILE *f = fopen("sweep.csv", "w");
    int n = 0, good = 0, near = 0, unmatched = 0;
    double worst_gap = 0;
    fprintf(f, "MHz,R,X,start_swr,tuned_swr,best_swr,adc_reads,seconds\n");
    fwd_mv = 900;
    for (unsigned i = 0; i < sizeof mhz / sizeof *mhz; i++)
        for (unsigned j = 0; j < sizeof r / sizeof *r; j++)
            for (unsigned k = 0; k < sizeof x / sizeof *x; k++) {
                freq = mhz[i] * 1e6;
                z_load = r[j] + x[k] * I;
                if (getenv("LOAD")) {
                    char want[40];
                    snprintf(want, sizeof want, "%.2f,%.0f,%.0f", mhz[i], r[j], x[k]);
                    if (strcmp(want, getenv("LOAD")) != 0)
                        continue;
                }
                double best = swr_best();
                if (best > 1.5)
                    continue; // the relays cannot match it anyway
                double start = swr_for(0, 0, 1);
                long reads = adc_reads;
                double t = secs();
                tune();
                double tuned = swr_now();
                if (tuned > 99) tuned = 99; // a real meter stops at 9.99
                fprintf(f, "%.2f,%.0f,%.0f,%.2f,%.2f,%.2f,%ld,%.2f\n", mhz[i], r[j], x[k], start, tuned, best,
                        adc_reads - reads, secs() - t);
                n++;
                if (tuned < 1.5) good++;
                if (tuned < best * 1.1 + 0.05) near++;
                if (tuned >= 2.0) unmatched++;
                if (tuned - best > worst_gap) worst_gap = tuned - best;
            }
    fclose(f);
    printf("  %d loads the relays can match below SWR 1.5\n", n);
    printf("  tuned below 1.5:            %d (%.1f%%)\n", good, 100.0 * good / n);
    printf("  within 10%% of the best:     %d (%.1f%%)\n", near, 100.0 * near / n);
    printf("  left at SWR 2 or more:      %d (%.1f%%)\n", unmatched, 100.0 * unmatched / n);
    printf("  worst gap to the best:      %.2f\n", worst_gap);
    exit(0);
}

static void scenario(void)
{
    char what[80];
    static int in_sweep;
    if (getenv("SWEEP")) {
        if (!in_sweep && secs() > 4) {
            in_sweep = 1;
            sweep();
        }
        return;
    }
    switch (step) {
    case 0: // boot
        if (!waited(4))
            break;
        check(got("\"Event\":\"Startup\""), "startup event");
        check(got("\"Build\":"), "board information");
        host_show_display("after boot");
        next();
        send("{\"ESP32\": \"Info\"}\n{\"Status\": true}\n"); // atutop's ESP32 probe, then status
        break;

    case 1:
        if (got("\"SWR\":")) {
            check(got("\"Auto\":false"), "status message answers {\"Status\": true}");
            check(!got("\"Event\":\"Error\""), "an ESP32 bridge probe is ignored without an Error");
            next();
            send("{\"Tune\": true}\n"); // no carrier yet
        } else
            timeout(2, "status");
        break;

    case 2:
        if (got("\"Event\":\"Tune\"") && waited(1.5)) {
            printf("  carrier on, antenna 35+j30 ohm, SWR %.2f\n", swr_now());
            fwd_mv = 900;
            next();
        } else
            timeout(3, "Tune event");
        break;

    case 3:
        if (got_status_after("\"Event\":\"Tuned\"")) {
            check(!got("\"Event\":\"Reset\""), "serial tune waits for the carrier without aborting");
            snprintf(what, sizeof what, "tuned: model SWR %.2f, reported %.2f", swr_now(), last_value("SWR"));
            check(swr_now() < 1.5 && last_value("SWR") < 1.5, what);
            next();
        } else
            timeout(60, "Tuned event");
        break;

    case 4:
        if (waited(2)) {
            host_show_display("carrier on after tune");
            check_display(display_has(1, "SWR=1."), "display shows SWR");
            check(strstr(from_pic, "\"Power\":10.") && strstr(from_pic, "\"Antenna\":") && strstr(from_pic, "\"Efficiency\":"),
                  "status has transmitter power, antenna power and efficiency");
            press(2, 0.3); // bypass button
            next();
        }
        break;

    case 5:
        if (got("\"Bypass\":true")) {
            check(relays_l() == 0 && relays_c() == 0, "bypass button releases all relays");
            check_display(display_has(0, "_"), "display shows the bypass mark");
            check(last_value("Efficiency") < 70, "efficiency follows the SWR into bypass");
            check(got("\"Capacitance\":0,\"Inductance\":0"), "status reports the released relays in bypass");
            host_show_display("bypass by button");
            next();
            send("{\"Bypass\": false}\n");
        } else
            timeout(3, "bypass from the button");
        break;

    case 6:
        if (got("\"Bypass\":false")) {
            check(swr_now() < 1.5, "leaving bypass by serial restores the tune");
            fwd_mv = 0;
            next();
            send("a\n");
        } else
            timeout(3, "bypass off by serial");
        break;

    case 7:
        if (got("\"Auto\":true")) {
            check(eeprom[2] == 1, "auto mode saved in eeprom");
            check_display(display_has(0, "W."), "display shows the auto mark");
            next();
            press(1, 0.3); // auto button turns it off again
        } else
            timeout(3, "auto on by serial letter");
        break;

    case 8:
        if (got("\"Auto\":false")) {
            next();
            send("{\"RelayI\": 3}{\"RelayC\": 129}\n");
        } else
            timeout(3, "auto off by button");
        break;

    case 9:
        if (got("\"Capacitance\":10,\"Inductance\":160")) {
            check(relays_l() == 3 && relays_c() == 1 && LATC_r.LATC0 == 1, "relays follow RelayI/RelayC");
            check(got("\"Order\":\"LC\""), "bit 7 of RelayC puts C on the output side");
            host_show_display("relays set by serial");
            next();
            send("{\"Tune\": true}\n"); // no carrier: waits
        } else
            timeout(3, "relay status");
        break;

    case 10: { // a tune waiting for carrier answers polls; Reset stops it
        static int sub;
        int busy = 0;
        for (const char *p = from_pic + mark; (p = strstr(p, "\"Event\":\"Busy\"")) != NULL; p++)
            busy++;
        if (sub == 0 && waited(2)) {
            send("{\"Status\": true}\n");
            sub = 1;
        } else if (sub == 1 && got("\"SWR\":")) {
            check(!got("\"Event\":\"Reset\"") && !got("\"Event\":\"Tuned\""),
                  "status answered while a tune waits for carrier");
            send("{\"Raw\": true}\n");
            sub = 2;
        } else if (sub == 2 && got("\"Fwd\":")) {
            check(!got("\"Event\":\"Reset\"") && busy == 0, "detector readings answered while waiting");
            send("{\"Bypass\": true}\n{\"Get\": 5}\n");
            sub = 3;
        } else if (sub == 3 && busy == 2) {
            check(!got("\"Event\":\"Reset\"") && !got("\"Bypass\":true") && !got("\"05\":"),
                  "other messages get Busy and the tune keeps waiting");
            send("{\"Reset\": true}\n");
            sub = 4;
        } else if (sub == 4 && got("\"Event\":\"Tuned\"")) {
            check(got("\"Event\":\"Reset\"") && relays_l() == 0 && relays_c() == 0,
                  "Reset message cancels a tune waiting for carrier");
            check(!got("\"Event\":\"Error\""), "no errors from messages during the tune");
            sub = 0;
            next();
            send("{\"Dump\": \"all\"}\n");
        } else
            timeout(15, "messages during a waiting tune");
        break;
    }

    case 11:
        if (waited(1) && step_time > 0) {
            send("{\"Status\": true}\n"); // while the dump is still being sent
            step_time = -secs();
        }
        if (got("\"ff\":") && got("\"Power\":")) {
            int cells = 0;
            const char *dump = strstr(from_pic + mark, "{\"00\":"), *end = strchr(dump, '\n');
            for (const char *p = dump; (p = strstr(p, "\":\"")) != NULL && p < end; p++)
                cells++;
            snprintf(what, sizeof what, "eeprom dump has 256 cells (got %d)", cells);
            check(cells == 256, what);
            check(1, "full duplex: status request handled during the dump");
            next();
            fwd_mv = 900;
            z_load = 80 - 60 * I;
            press(0, 0.15); // short press: reset
        } else if (step_time < 0 && secs() + step_time > 15)
            timeout(0, "dump and status");
        break;

    case 12:
        if (got("\"Event\":\"Reset\"") && waited(1)) {
            check(relays_l() == 0 && relays_c() == 0, "short press of Tune resets the relays");
            printf("  antenna now 80-j60 ohm, SWR %.2f\n", swr_now());
            check(!got("\"Event\":\"HighSWR\""), "no HighSWR alarm below the default 3.0");
            next();
            send("{\"Raw\": true}\n{\"Set\": 55, \"Value\": 32}\n"); // alarm at SWR 2.0
        } else
            timeout(5, "reset by button");
        break;

    case 125:
        if (got("\"Fwd\":") && got("\"Event\":\"HighSWR\"")) {
            check(last_value("Fwd") > 2500 && last_value("Rev") > 1000, "raw detector readings");
            check(eeprom[0x37] == 0x20, "alarm limit written");
            check(1, "new alarm limit applies without a restart");
            next();
            step = 13;
            press(0, 0.8); // long press: tune
        } else
            timeout(5, "raw readings and HighSWR");
        break;

    case 13:
        if (got_status_after("\"Event\":\"Tuned\"")) {
            snprintf(what, sizeof what, "button tune: model SWR %.2f, reported %.2f", swr_now(), last_value("SWR"));
            check(swr_now() < 1.5 && last_value("SWR") < 1.5, what);
            host_show_display("tuned by button");
            next();
        } else
            timeout(60, "button tune");
        break;

    case 14:
        if (waited(2)) {
            check(!got("\"Event\":\"HighPower\""), "no HighPower alarm at 10 W");
            fwd_mv = 3300; // about 120 W into the stock 1:10 winding
            next();
        }
        break;

    case 15:
        if (got("\"Event\":\"HighPower\"")) {
            check(!got("\"Event\":\"Overload\""), "HighPower alarm above the board's 100 W, before the detector overloads");
            fwd_mv = 900;
            next();
        } else
            timeout(3, "HighPower event");
        break;

    case 16:
        if (waited(1))
            finish();
        break;
    }
}

int main(void)
{
    double err = getenv("BAUD_ERR") ? atof(getenv("BAUD_ERR")) : 0;
    srand(1);
    bit_cycles = 832 * (1 + err / 100); // 16 MHz / 16 / 208 = 4807.7 baud = 832 PIC cycles
    memcpy(eeprom, initial_eeprom, sizeof eeprom);
    if (getenv("DISPLAY_TYPE"))
        eeprom[1] = (unsigned char)atoi(getenv("DISPLAY_TYPE"));
    display_type = eeprom[1];
    if (getenv("TUNE_MODE"))
        eeprom[0x36] = (unsigned char)atoi(getenv("TUNE_MODE"));
    misc_r.nTO = 1;
    PORTB_r.v = 0x07; // buttons up
    printf("ATU-100 host test: display type %d, Arduino baud error %+.1f%%\n", display_type, err);
    if (getenv("PTY")) {
        /* The serial line on a pseudo terminal, in real time, with a carrier
           into a 35+j30 ohm antenna, for testing the python tools */
        setvbuf(stdout, NULL, _IOLBF, 0);
        pty = posix_openpt(O_RDWR | O_NOCTTY);
        grantpt(pty);
        unlockpt(pty);
        fcntl(pty, F_SETFL, O_NONBLOCK);
        // Raw mode, held by keeping the other side open: no echo of what
        // the tuner sends back into the tuner, no newline translation
        int side = pty_side = open(ptsname(pty), O_RDWR | O_NOCTTY);
        struct termios tio;
        tcgetattr(side, &tio);
        cfmakeraw(&tio);
        tcsetattr(side, TCSANOW, &tio);
        printf("serial port: %s\n", ptsname(pty));
        FILE *f = fopen("pty_name", "w");
        fprintf(f, "%s\n", ptsname(pty));
        fclose(f);
        fflush(stdout);
        fwd_mv = getenv("NO_RF") ? 0 : 900;
    }
    fw_main();
    return 0;
}
