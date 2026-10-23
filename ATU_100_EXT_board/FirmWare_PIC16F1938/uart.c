/*  uart.c
    Interrupt driven software UART, 4800 baud 8N1.
    The PIC's hardware UART pins drive capacitor relays, so the bits are
    timed in software on the pins named in cross_compiler.h.

    Timer2 interrupts at 4 times the bit rate: 4 MHz / 208 = 19231 Hz,
    giving 4808 baud (0.16% fast, the same as an Arduino at 4800).
    Receive samples the middle of each bit; transmit and receive are
    independent so both directions can run at once. */

#include "uart.h"

#define TICKS_PER_BIT 4
#define RX_LEN 128 /* both lengths must be powers of 2 */
#define TX_LEN 128

static volatile uint8_t rx_buf[RX_LEN], rx_in, rx_out;
static volatile uint8_t tx_buf[TX_LEN], tx_in, tx_out;
static volatile uint8_t rx_messages; // '{' received and not yet read

void uart_init(void) {
    UART_OUT_PIN = 1; // idle line is high
    UART_OUT_TRIS = 0;
    UART_IN_TRIS = 1;
    PR2 = 207; // 208 instruction cycles per tick
    T2CON = 0b00000100; // Timer2 on, prescaler and postscaler 1:1
    PIE1bits.TMR2IE = 1;
    INTCONbits.PEIE = 1;
    INTCONbits.GIE = 1;
}

void __interrupt() isr(void) {
    static uint8_t rx_bits, rx_cnt, rx_byte;
    static uint8_t tx_bits, tx_cnt, tx_byte;

    if (!PIR1bits.TMR2IF)
        return;
    PIR1bits.TMR2IF = 0;

    // Receive. rx_bits: 10 = start bit, 9..2 = data bits, 1 = stop bit
    if (rx_bits == 0) {
        if (UART_IN_PIN == 0) { // falling edge of a start bit
            rx_bits = 10;
            rx_cnt = TICKS_PER_BIT / 2; // check it again in its middle
        }
    } else if (--rx_cnt == 0) {
        rx_cnt = TICKS_PER_BIT;
        if (rx_bits == 10) {
            rx_bits = UART_IN_PIN ? 0 : 9; // high again: just a glitch
        } else if (rx_bits > 1) {
            rx_byte >>= 1;
            if (UART_IN_PIN)
                rx_byte |= 0x80;
            rx_bits--;
        } else {
            if (UART_IN_PIN && ((rx_in + 1) & (RX_LEN - 1)) != rx_out) {
                rx_buf[rx_in] = rx_byte;
                rx_in = (rx_in + 1) & (RX_LEN - 1);
                if (rx_byte == '{')
                    rx_messages++;
            }
            rx_bits = 0;
        }
    }

    // Transmit. tx_bits: 8..1 = data bits to go, 0 = stop bit
    if (tx_cnt) {
        tx_cnt--;
    } else if (tx_bits) {
        if (--tx_bits) {
            UART_OUT_PIN = tx_byte & 1;
            tx_byte >>= 1;
        } else {
            UART_OUT_PIN = 1;
        }
        tx_cnt = TICKS_PER_BIT - 1;
    } else if (tx_out != tx_in) {
        tx_byte = tx_buf[tx_out];
        tx_out = (tx_out + 1) & (TX_LEN - 1);
        UART_OUT_PIN = 0; // start bit
        tx_bits = 9;
        tx_cnt = TICKS_PER_BIT - 1;
    }
}

void uart_putc(char c) {
    uint8_t next = (tx_in + 1) & (TX_LEN - 1);
    while (next == tx_out) // buffer full: wait for the interrupt to send some
        CLRWDT();
    tx_buf[tx_in] = c;
    tx_in = next;
}

void uart_str(const char *str) {
    while (*str)
        uart_putc(*str++);
}

// Has a new JSON message started arriving? (Counting the opening brace:
// a command is acted on before its closing brace has been read.)
bool uart_rx_message(void) {
    return rx_messages != 0;
}

// Is c waiting in the receive buffer?
bool uart_rx_has(char c) {
    uint8_t i;
    for (i = rx_out; i != rx_in; i = (i + 1) & (RX_LEN - 1))
        if (rx_buf[i] == c)
            return true;
    return false;
}

char uart_getc(void) {
    char c;
    if (rx_in == rx_out)
        return 0;
    c = rx_buf[rx_out];
    rx_out = (rx_out + 1) & (RX_LEN - 1);
    if (c == '{')
        rx_messages--;
    return c;
}
