/*  Stand-in for XC8's xc.h so the firmware compiles with gcc on a PC.
    Registers are plain variables; host.c moves simulated time forward,
    runs the interrupt and plays the radio, buttons and serial partner. */
#ifndef FAKE_XC_H
#define FAKE_XC_H

#include <stdint.h>

#define BITS8(p) unsigned p##0 : 1, p##1 : 1, p##2 : 1, p##3 : 1, p##4 : 1, p##5 : 1, p##6 : 1, p##7 : 1

typedef union { uint8_t v; struct { BITS8(RA); }; } porta_t;
typedef union { uint8_t v; struct { BITS8(RB); }; } portb_t;
typedef union { uint8_t v; struct { BITS8(RC); }; } portc_t;
typedef union { uint8_t v; struct { BITS8(LATA); }; } lata_t;
typedef union { uint8_t v; struct { BITS8(LATB); }; } latb_t;
typedef union { uint8_t v; struct { BITS8(LATC); }; } latc_t;
typedef union { uint8_t v; struct { BITS8(TRISA); }; } trisa_t;
typedef union { uint8_t v; struct { BITS8(TRISB); }; } trisb_t;
typedef union { uint8_t v; struct { BITS8(TRISC); }; } trisc_t;

extern volatile porta_t PORTA_r;
extern volatile portb_t PORTB_r;
extern volatile portc_t PORTC_r;
extern volatile lata_t LATA_r;
extern volatile latb_t LATB_r;
extern volatile latc_t LATC_r;
extern volatile trisa_t TRISA_r;
extern volatile trisb_t TRISB_r;
extern volatile trisc_t TRISC_r;

#define PORTA PORTA_r.v
#define PORTB PORTB_r.v
#define PORTC PORTC_r.v
#define LATA LATA_r.v
#define LATB LATB_r.v
#define LATC LATC_r.v
#define TRISA TRISA_r.v
#define TRISB TRISB_r.v
#define TRISC TRISC_r.v
#define PORTAbits PORTA_r
#define PORTBbits PORTB_r
#define PORTCbits PORTC_r
#define LATAbits LATA_r
#define LATBbits LATB_r
#define LATCbits LATC_r
#define TRISAbits TRISA_r
#define TRISBbits TRISB_r
#define TRISCbits TRISC_r

struct misc_bits {
    unsigned TMR2IF : 1, TMR2IE : 1, GIE : 1, PEIE : 1, TXIF : 1, SPEN : 1, TXEN : 1, nTO : 1;
    unsigned WDTPS0 : 1, WDTPS1 : 1, WDTPS2 : 1, WDTPS3 : 1, WDTPS4 : 1;
    unsigned ANSA0 : 1, ANSA1 : 1, C1ON : 1, C2ON : 1, nWPUEN : 1, WPUB0 : 1, WPUB1 : 1, WPUB2 : 1;
    unsigned ADPREF0 : 1, ADPREF1 : 1, ADCS0 : 1, ADCS1 : 1, ADCS2 : 1, ADON : 1, ADGO : 1;
    unsigned CHS : 5, FVRRDY : 1;
    uint8_t ANSELA, ANSELB;
};
extern struct misc_bits misc_r;
struct misc_bits *adc_hook(void); // starting a conversion finishes it

#define PIR1bits misc_r
#define PIE1bits misc_r
#define INTCONbits misc_r
#define TXSTAbits misc_r
#define RCSTAbits misc_r
#define STATUSbits misc_r
#define WDTCONbits misc_r
#define ANSELAbits misc_r
#define ANSELBbits misc_r
#define CM1CON0bits misc_r
#define CM2CON0bits misc_r
#define OPTION_REGbits misc_r
#define WPUBbits misc_r
#define ADCON1bits misc_r
#define FVRCONbits misc_r
#define ADCON0bits (*adc_hook())

extern uint8_t OSCCON, FVRCON, ADCON1, ADRESL, ADRESH, PR2, T2CON, TXREG;

void host_delay(unsigned long cycles);
void host_clrwdt(void);
void host_reset(void);
unsigned char eeprom_read(unsigned char addr);
void eeprom_write(unsigned char addr, unsigned char value);

#define _delay(x) host_delay(x)
#define CLRWDT() host_clrwdt()
#define RESET() host_reset()
#define __interrupt()
#define __eeprom

#endif
