
#include "cross_compiler.h"
#include "uart.h"

// Main.h
// David Fainitski
// ATU-100 project 2016

//
static char g_c_ind = 0, g_c_cap = 0;
static char g_c_SW = 0;
static char g_c_L_mult = 1, g_c_C_mult = 1;
int g_i_SWR, g_i_PWR, g_i_P_max, g_i_swr_a;
static char g_b_rready = 0, g_char_p_cnt = 0;
// A tune given up while waiting for carrier: reset once the search has
// returned (tune_btn_push), not down here, where the call stack is deep
static char g_b_tune_given_up = 0;

static char g_b_Overload = 0;
static unsigned int g_u_fwd, g_u_rev; // get_pwr's raw readings, for the model search

/* relay values in pF and nH, from eeprom cells 0x20 and 0x10 */
static int e_i_Cap[7];
static int e_i_Ind[7];
/* booleans indicating whether the inductors and caps are linearly spaced */
static char e_c_b_L_linear = 0, e_c_b_C_linear = 0;
/* number of inductors and caps */
static char e_c_num_L_q = 7, e_c_num_C_q = 7;
/*  boolean indicating whether we should correct for the diode */
static char e_c_b_D_correction = 1;
/*  boolean indicating if the inductors are using relays with normally open contact */
static char e_c_b_L_invert = 0;
/* in automatic mode, threshold triggering the tuning process if the 
 * SWR is above this level.   in tenths.   so 13 would be an SWR of 1.3 */
static int e_i_tenths_SWR_Auto_delta;
/* display off timer: 0 = always on */
static char e_c_b_Dysp_delay = 0;
/* feeder loss in tenths of a db.  */
static char e_c_tenths_Fid_loss;
/* delay in ms for relays to settle and to take a measurement */
static int e_i_ms_Rel_Del;
/*  power in watts for starting tuning.  min and max */
static int e_i_watts_min_for_start, e_i_watts_max_for_start;
/*  max SWR for tuning, in tenths.  0 = do not check */
static int e_i_tenths_init_max_swr;
/* boolean support high power measurements */
static char e_c_b_P_High = 0;
/* ratio of turns of the tandem match (default should be 10) */
static char e_c_K_Mult = 32;
/* bool indicating whether to display Power level also */
static char e_c_b_Loss_ind = 0;
/* boolean.  to support Relay off function */
static char e_c_b_Relay_off = 0;

//
void pic_init(void);

void tune_btn_push(void);
char serial_during_tune(void);
void lcd_prep(void);
void lcd_swr(int);
void lcd_pwr(void);
void show_pwr(int, int);
void lcd_ind(void);
void show_reset(void);
void cells_init(void);
void test_init(void);
void button_proc(void);
void button_proc_test(void);
void button_delay(void);
void show_loss(void);
//
void atu_reset(void);
unsigned int get_reverse(void);
unsigned int get_forward(void);
int correction(int);
void get_swr(void);
void get_pwr(void);
void set_sw(char);
void tune(void);
//

/*  The calculator. Every float operation costs the PIC 20 to 40 words of
    code, so the formulas are byte programs for one small interpreter: an
    accumulator, 16 registers (t_v) and 16 constants (m_k). An operand op
    is (op << 4 | register), or | 0x80 for a constant. POS ends the
    program, failed, unless the accumulator is above 0; OUT puts it into
    t_out, as an int cut towards 0 (ADD 0.5 first to round; the program
    keeps it within an int, with MIN K_WMAX). The registers
    are scratch: nothing is kept in them from one measurement to the next.
    research/tuning/firmware.py reads these programs from this file. */
#define LD(r) (0x00 | (r))
#define ADD(r) (0x10 | (r))
#define SUB(r) (0x20 | (r))
#define MUL(r) (0x30 | (r))
#define DIV(r) (0x40 | (r))
#define MIN(r) (0x50 | (r))
#define MAX(r) (0x60 | (r))
#define ST(r) (0x70 | (r))
#define K(op) (0x80 | (op))
#define END 0xF0
#define POS 0xF1
#define SQRT 0xF2
#define NEG 0xF3
#define OUT 0xF4

// Constants
#define K_ONE 0
#define K_HALF 1
#define K_CENTI 2
#define K_RMAX 3
#define K_SIG_A 4
#define K_SIG_B 5
#define K_SIG_C 6
#define K_16 7
#define K_WMAX 8
#define K_1000 9
#define K_04 10
#define K_FMIN 11
#define K_FMAX 12
#define K_3 13
#define K_VOLT 14
#define K_LOSS 15
static const float m_k[16] = {1, 0.5, 0.01, 0.995, 0.000144, 0.04, 0.000064, 16,
    30000, 1000, 0.4, 0.0377, 10.05, 3, 1 / 1414.0, 0.0115};

static float t_v[16];
static int t_out[3];

static void m_in(char r, int v) {
    t_v[r] = v;
}

/*  Newton from a first guess within 6%: the IEEE 754 exponent halved.
    Three steps reach the float's precision */
static float m_sqrt(float x) {
    float y;
    char i;
    *(uint32_t *) &y = (*(uint32_t *) &x >> 1) + 0x1FC00000;
    for (i = 0; i < 3; i++)
        y = (y + x / y) * 0.5;
    return y;
}

/*  -x: flips the sign bit (IEEE 754, the high byte last; XC8's floats and
    the PC's alike), a few words where x = -x calls the library */
#define M_NEG(x) (((unsigned char *) &(x))[3] ^= 0x80)

// Run a program; 0 when a POS failed
static char m_calc(const char *p) {
    float a = 0, b;
    char op, i, n = 0;
    for (;;) {
        op = *p++;
        i = op & 15;
        b = op & 0x80 ? m_k[i] : t_v[i];
        switch (op >> 4 & 7) {
            case 0: a = b;
                break;
            case 2: // a - b is a + (-b)
                M_NEG(b);
                // fall through
            case 1: a += b;
                break;
            case 3: a *= b;
                break;
            case 4: a /= b;
                break;
            case 5: // MIN
            case 6: // MAX
                if ((b < a) == !(op & 0x20))
                    a = b;
                break;
            default:
                if (!(op & 0x80))
                    t_v[i] = a;
                else if (i == 0)
                    return 1;
                else if (i == 1) {
                    if (!(a > 0))
                        return 0;
                } else if (i == 2)
                    a = m_sqrt(a); // never given less than 0
                else if (i == 3)
                    M_NEG(a);
                else // the programs keep it within an int
                    t_out[n++] = (int) a;
        }
    }
}

/*  Watts from the forward reading: (mV * K_Mult / 1414)^2 / R, R 50 for
    watts or 5 for tenths; registers 0, 1, 2 */
static const char p_power[] = {LD(0), MUL(1), K(MUL(K_VOLT)), ST(3), MUL(3), DIV(2), K(ADD(K_HALF)), OUT, END};
/*  Antenna power and efficiency (in t_out) from the feeder loss and the
    SWR: a = 1 / ((S + 1 / S) loss 0.0115 + 1), S the SWR at the tuner's
    input before the tune (register 0, x100) and loss in tenths of a dB
    (1); b = 4 / (2 + S + 1 / S), S the SWR now (2, x100); both at most
    1; power in 3 */
static const char p_loss[] = {LD(0), K(MUL(K_CENTI)), ST(4), K(LD(K_ONE)), DIV(4), ADD(4), MUL(1),
    K(MUL(K_LOSS)), K(ADD(K_ONE)), ST(4), K(LD(K_ONE)), DIV(4), K(MIN(K_ONE)), ST(5),
    LD(2), K(MUL(K_CENTI)), ST(4), K(LD(K_ONE)), DIV(4), ADD(4), K(ADD(K_ONE)), K(ADD(K_ONE)), ST(4),
    K(LD(K_16)), K(MUL(K_HALF)), K(MUL(K_HALF)), DIV(4), K(MIN(K_ONE)), MUL(5), ST(5),
    MUL(3), OUT, LD(5), K(DIV(K_CENTI)), OUT, END};

// N7DDC's diode correction: up to each reading ("x3" mV), what to add
static const int corr_upto[] = {171, 328, 582, 820, 1100, 2181, 3322, 4623, 5862, 7146, 8502, 10500};
static const int corr_add[] = {244, 254, 280, 297, 310, 430, 484, 530, 648, 743, 800, 840, 860};

int correction(int input) {
    char i;
    if (input <= 80)
        return 0;
    for (i = 0; i < 12 && input > corr_upto[i]; i++)
        ;
    return input + corr_add[i];
}

//

unsigned int get_reverse() {
    unsigned int returnReverse;
    FVRCON = 0b10000001; // ADC 1024 vmV Vref

    WAIT_FOR_FVR

    returnReverse = ADC_Get_Sample(0);
    if (returnReverse <= 1000)
        return returnReverse;
    FVRCON = 0b10000010; // ADC 2048 vmV Vref

    WAIT_FOR_FVR

    returnReverse = ADC_Get_Sample(0);
    if (returnReverse <= 1000)
        return returnReverse * 2;
    FVRCON = 0b10000011; // ADC 4096 vmV Vref

    WAIT_FOR_FVR

    returnReverse = ADC_Get_Sample(0);
    return returnReverse * 4;
}
//

unsigned int get_forward() {
    unsigned int returnUIntValue;
    FVRCON = 0b10000001; // ADC 1024 vmV Vref

    WAIT_FOR_FVR

    returnUIntValue = ADC_Get_Sample(1);
    if (returnUIntValue <= 1000) {
        g_b_Overload = 0;
        return returnUIntValue;
    }
    FVRCON = 0b10000010; // ADC 2048 vmV Vref

    WAIT_FOR_FVR

    returnUIntValue = ADC_Get_Sample(1);
    if (returnUIntValue <= 1000) {
        g_b_Overload = 0;
        return returnUIntValue * 2;
    }
    FVRCON = 0b10000011; // ADC 4096 vmV Vref

    WAIT_FOR_FVR

    returnUIntValue = ADC_Get_Sample(1);
    if (returnUIntValue > 1000)
        g_b_Overload = 1;
    else
        g_b_Overload = 0;
    return returnUIntValue * 4;
}

void get_pwr() {
    long l_Forward, l_Reverse;
    CLRWDT();
    //
    l_Forward = g_u_fwd = get_forward();
    l_Reverse = g_u_rev = get_reverse();
    m_in(0, e_c_b_D_correction == 1 ? correction((int) (l_Forward * 3)) : l_Forward * 3);
    m_in(1, e_c_K_Mult);
    m_in(2, e_c_b_P_High == 1 ? 50 : 5); // 0 - 1500 W, or 0 - 151.0 W
    m_calc(p_power);
    //
    if (l_Reverse >= l_Forward)
        l_Forward = 999;
    else {
        l_Forward = ((l_Forward + l_Reverse) * 100) / (l_Forward - l_Reverse);
        if (l_Forward > 999)
            l_Forward = 999;
    }
    //
    g_i_PWR = t_out[0];
    if (l_Forward < 100)
        g_i_SWR = 999;
    else
        g_i_SWR = (int) (l_Forward);
    return;
}

void get_swr() {
    uint32_t timeout = 0;

    get_pwr();
    if (g_char_p_cnt != 100) {
        g_char_p_cnt += 1;
        if (g_i_PWR > g_i_P_max)
            g_i_P_max = g_i_PWR;
    } else {
        g_char_p_cnt = 0;
        show_pwr(g_i_P_max, g_i_SWR);
        g_i_P_max = 0;
    }
    while ((g_i_PWR < e_i_watts_min_for_start) | (g_i_PWR > e_i_watts_max_for_start & e_i_watts_max_for_start > 0)) { // waiting for good power
        CLRWDT();
        get_pwr();
        if (g_char_p_cnt != 100) {
            g_char_p_cnt += 1;
            if (g_i_PWR > g_i_P_max)
                g_i_P_max = g_i_PWR;
        } else {
            g_char_p_cnt = 0;
            show_pwr(g_i_P_max, g_i_SWR);
            g_i_P_max = 0;
        }
        //
        if (Button(&PORTB, TUNE_BUTTON, 5, BUTTON_RELEASED))
            g_b_rready = 1;

        // Give up after about a minute, on the Tune button, or on a Reset
        // or Tune message (other messages are answered while waiting)
        if ((timeout++ > 7200) || serial_during_tune() || ((g_b_rready == 1) & Button(&PORTB, TUNE_BUTTON, 5, BUTTON_PRESSED))) {
            // show_reset() from here went 17 calls deep with the OLED, past
            // the PIC's 16-level stack: the chip restarted
            g_b_tune_given_up = 1;
            g_i_SWR = 0;
            return;
        }
    } //  good power
    return;
}

void set_ind(char Ind) {
    charbits Indbits;
    Indbits.bytes = Ind;

    if (e_c_b_L_invert == 0) {
        Ind_005 = Indbits.bits.B0;
        Ind_011 = Indbits.bits.B1;
        Ind_022 = Indbits.bits.B2;
        Ind_045 = Indbits.bits.B3;
        Ind_1 = Indbits.bits.B4;
        Ind_22 = Indbits.bits.B5;
        Ind_45 = Indbits.bits.B6;
        //
    } else {
        Ind_005 = ~Indbits.bits.B0;
        Ind_011 = ~Indbits.bits.B1;
        Ind_022 = ~Indbits.bits.B2;
        Ind_045 = ~Indbits.bits.B3;
        Ind_1 = ~Indbits.bits.B4;
        Ind_22 = ~Indbits.bits.B5;
        Ind_45 = ~Indbits.bits.B6;
        //
    }
    Vdelay_ms(e_i_ms_Rel_Del);
}

void set_cap(char Cap) {
    charbits Capbits;
    Capbits.bytes = Cap;

    Cap_10 = Capbits.bits.B0;
    Cap_22 = Capbits.bits.B1;
    Cap_47 = Capbits.bits.B2;
    Cap_100 = Capbits.bits.B3;
    Cap_220 = Capbits.bits.B4;
    Cap_470 = Capbits.bits.B5;
    Cap_1000 = Capbits.bits.B6;
    //
    Vdelay_ms(e_i_ms_Rel_Del);
}

void set_sw(char g_c_SW) { // 0 - IN,  1 - OUT
    Cap_sw = g_c_SW;
    Vdelay_ms(e_i_ms_Rel_Del);
}

void atu_reset() {
    g_c_ind = 0;
    g_c_cap = 0;
    set_ind(g_c_ind);
    set_cap(g_c_cap);
    Vdelay_ms(e_i_ms_Rel_Del);
}

/*  Model search: measure, calculate, verify (research/tuning/firmware.py runs
    this same search in the lab, and README.md tells how it was chosen).

    With C = 0 the network adds only a series L to the antenna Z = R + jX,
    and s = (1 + |g|^2) / (1 - |g|^2) is exactly a parabola in L:
        s = (R^2 + (X + wL)^2 + 2500) / (100 R)
    With L = 0 the same holds for C in admittance. A parabola fitted to the
    readings along each line gives R, X and the frequency (two answers, R
    and 2500 / R), and from them the relays of an exact match on either
    capacitor side. Those are measured, the best refined with a bowl
    fitted to 3x3 patches, then the bowl again from the best point with
    steps of 2 and 1 relays. When that ends above
    SWR 1.3: a 7x7 grid on both sides, and the same refining from its best
    three points.

    Units: 50 ohm (and 1/50 S) is 1, L in uH, C in nF; w is the series
    reactance of 1 uH, so 1 nF has 2.5 w. */

/*  Registers while measuring: the line's sums (sum w x^k, k 0..4, and
    sum w s x^k, k 0..2), then the reading */
#define R_M0 0
#define R_M1 1
#define R_M2 2
#define R_M3 3
#define R_M4 4
#define R_V0 5
#define R_V1 6
#define R_V2 7
#define R_W 8
#define R_S 9
#define R_X 10
#define R_T 11
#define R_R 12
#define R_F 13
#define R_CF 14
#define R_R2 14
#define R_G2 15 // read it straight after m_measure: later programs use it
#define R_Q 8 // the weight's register, before the weight

// |g| = R / CF, the smallest the reflected diode shows F = F / CF, and |g|^2
static const char p_rho[] = {LD(R_R), DIV(R_CF), K(MIN(K_RMAX)), ST(R_R), MUL(R_R), ST(R_G2),
    LD(R_F), DIV(R_CF), ST(R_F), END};
// How uncertain |g| is: 1.2%, the diode's table, the ADC's steps (squared, in T)
static const char p_sig_read[] = {LD(R_R), MUL(R_R), ST(R_R2), K(MUL(K_SIG_A)), ST(R_T),
    LD(R_F), MUL(R_F), K(MUL(K_SIG_B)), ADD(R_T), K(ADD(K_SIG_C)), ST(R_T), END};
// A reading of 0: |g| is somewhere below F, taken as F / 2 with that much doubt
static const char p_sig_knee[] = {LD(R_R), MUL(R_R), ST(R_R2), LD(R_F), MUL(R_F),
    K(MUL(K_HALF)), K(MUL(K_HALF)), ST(R_T), END};
/*  s, its weight w = 1 / sigma_s^2 (sigma_s = 4 |g| / (1 - |g|^2)^2 sigma),
    and into the sums at x (given in nH or pF) */
static const char p_add[] = {LD(R_X), K(DIV(K_1000)), ST(R_X), K(LD(K_ONE)), SUB(R_R2), ST(R_Q), K(LD(K_ONE)), ADD(R_R2), DIV(R_Q),
    ST(R_S), LD(R_Q), MUL(R_Q), ST(R_Q), MUL(R_Q), ST(R_Q),
    LD(R_T), MUL(R_R2), K(MUL(K_16)), DIV(R_Q), ST(R_T), K(LD(K_ONE)), DIV(R_T), K(MIN(K_WMAX)),
    ST(R_W), ST(R_T),
    ADD(R_M0), ST(R_M0), LD(R_T), MUL(R_S), ADD(R_V0), ST(R_V0), LD(R_T), MUL(R_X), ST(R_T),
    ADD(R_M1), ST(R_M1), LD(R_T), MUL(R_S), ADD(R_V1), ST(R_V1), LD(R_T), MUL(R_X), ST(R_T),
    ADD(R_M2), ST(R_M2), LD(R_T), MUL(R_S), ADD(R_V2), ST(R_V2), LD(R_T), MUL(R_X), ST(R_T),
    ADD(R_M3), ST(R_M3), LD(R_T), MUL(R_X), ADD(R_M4), ST(R_M4), END};

/*  Registers while calculating: the sums, then the parabola
    s = c0 + c1 x + c2 x^2 by elimination (the sums make a positive
    definite matrix: no pivoting needed) */
#define R_L1 8
#define R_L2 9
#define R_A11 10
#define R_A12 11
#define R_B1 12
#define R_A22 13
#define R_B2 14
#define R_FT 15 // scratch (R_T is A12 here)
#define R_C0 9
#define R_C1 3
#define R_C2 4
static const char p_fit[] = {LD(R_M0), POS,
    LD(R_M1), DIV(R_M0), ST(R_L1), LD(R_M2), DIV(R_M0), ST(R_L2),
    LD(R_L1), MUL(R_M1), ST(R_FT), LD(R_M2), SUB(R_FT), POS, ST(R_A11),
    LD(R_L1), MUL(R_M2), ST(R_FT), LD(R_M3), SUB(R_FT), ST(R_A12),
    LD(R_L1), MUL(R_V0), ST(R_FT), LD(R_V1), SUB(R_FT), ST(R_B1),
    LD(R_L2), MUL(R_M2), ST(R_FT), LD(R_M4), SUB(R_FT), ST(R_A22),
    LD(R_L2), MUL(R_V0), ST(R_FT), LD(R_V2), SUB(R_FT), ST(R_B2),
    LD(R_A12), DIV(R_A11), ST(R_L1),
    LD(R_L1), MUL(R_A12), ST(R_FT), LD(R_A22), SUB(R_FT), POS, ST(R_A22),
    LD(R_L1), MUL(R_B1), ST(R_FT), LD(R_B2), SUB(R_FT), DIV(R_A22), POS, ST(R_C2),
    LD(R_A12), MUL(R_C2), ST(R_FT), LD(R_B1), SUB(R_FT), DIV(R_A11), ST(R_C1),
    LD(R_M1), MUL(R_C1), ST(R_FT), LD(R_V0), SUB(R_FT), ST(R_FT),
    LD(R_M2), MUL(R_C2), ST(R_L1), LD(R_FT), SUB(R_L1), DIV(R_M0), ST(R_C0), END};
// The bottom, x = -c1 / (2 c2), in nH or pF: 3/4 x, x and 4/3 x
static const char p_vertex[] = {LD(R_C1), DIV(R_C2), NEG, K(MUL(K_HALF)), POS, K(MUL(K_1000)),
    K(MIN(K_WMAX)), ST(R_T),
    K(MUL(K_HALF)), K(MUL(K_HALF)), ST(R_L1), LD(R_T), SUB(R_L1), ST(R_L1), K(ADD(K_HALF)), OUT,
    LD(R_T), K(ADD(K_HALF)), OUT, LD(R_T), DIV(R_L1), MUL(R_T), K(MIN(K_WMAX)), K(ADD(K_HALF)), OUT, END};

/*  The antenna: K = s at the bottom = c0 - c1^2 / (4 c2), at least 1, and
    R^2 - 2 K R + 1 = 0: R = K + Q or K - Q, Q = sqrt(K^2 - 1); then
    w = sqrt(2 R c2) and X = c1 R / w */
#define R_RE 0
#define R_IM 1
#define R_WW 2
#define R_K 5
#define R_QQ 6
#define R_G 7
#define R_B 8
#define R_SS 12
#define R_WC 13
#define R_WL 14
static const char p_roots[] = {LD(R_C1), MUL(R_C1), DIV(R_C2), K(MUL(K_HALF)), K(MUL(K_HALF)), ST(R_T),
    LD(R_C0), SUB(R_T), K(MAX(K_ONE)), ST(R_K), MUL(R_K), K(SUB(K_ONE)), SQRT, ST(R_QQ), END};
static const char p_plus[] = {LD(R_K), ADD(R_QQ), ST(R_RE), END};
static const char p_minus[] = {LD(R_K), SUB(R_QQ), POS, ST(R_RE), END};
static const char p_ant[] = {LD(R_RE), MUL(R_C2), ST(R_T), ADD(R_T), SQRT, ST(R_WW),
    LD(R_C1), MUL(R_RE), DIV(R_WW), ST(R_IM), END};
// From the C line: an admittance, to an impedance; and C's w to L's
static const char p_admit[] = {LD(R_RE), MUL(R_RE), ST(R_T), LD(R_IM), MUL(R_IM), ADD(R_T), ST(R_T),
    LD(R_RE), DIV(R_T), ST(R_RE), LD(R_IM), DIV(R_T), NEG, ST(R_IM),
    LD(R_WW), K(MUL(K_04)), ST(R_WW), END};
// 0.3 to 80 MHz, then the admittance G + jB
static const char p_check[] = {LD(R_WW), K(SUB(K_FMIN)), POS, K(LD(K_FMAX)), SUB(R_WW), POS,
    LD(R_RE), MUL(R_RE), ST(R_T), LD(R_IM), MUL(R_IM), ADD(R_T), ST(R_T),
    LD(R_RE), DIV(R_T), ST(R_G), LD(R_IM), DIV(R_T), NEG, ST(R_B), END};
/*  The exact L network. C at the antenna (LC) needs 0 < G < 1:
    S = +-sqrt(G (1 - G)), wC = S - B, wL = S / (G^2 + S^2).
    C at the transmitter (CL) needs 0 < R < 1:
    S = +-sqrt(R (1 - R)), wL = S - X, wC = S / (R^2 + S^2) */
static const char p_lc_s[] = {LD(R_G), POS, K(LD(K_ONE)), SUB(R_G), POS, MUL(R_G), SQRT, ST(R_SS), END};
static const char p_cl_s[] = {LD(R_RE), POS, K(LD(K_ONE)), SUB(R_RE), POS, MUL(R_RE), SQRT, ST(R_SS), END};
static const char p_neg_s[] = {LD(R_SS), NEG, ST(R_SS), END};
static const char p_lc[] = {LD(R_SS), SUB(R_B), POS, ST(R_WC), LD(R_G), MUL(R_G), ST(R_T),
    LD(R_SS), MUL(R_SS), ADD(R_T), ST(R_T), LD(R_SS), DIV(R_T), POS, ST(R_WL), END};
static const char p_cl[] = {LD(R_SS), SUB(R_IM), POS, ST(R_WL), LD(R_RE), MUL(R_RE), ST(R_T),
    LD(R_SS), MUL(R_SS), ADD(R_T), ST(R_T), LD(R_SS), DIV(R_T), POS, ST(R_WC), END};
// The relays' targets: L in nH, C in pF (at most 30000)
static const char p_target[] = {LD(R_WL), DIV(R_WW), K(MUL(K_1000)), K(MIN(K_WMAX)), K(ADD(K_HALF)), OUT,
    LD(R_WC), DIV(R_WW), K(MUL(K_1000)), K(MUL(K_04)), K(MIN(K_WMAX)), K(ADD(K_HALF)), OUT, END};

/*  A bowl along L and along C apart, from a 3x3 patch of |g|^2 in
    registers 0..8 (L steps -1, 0, 1 by rows): per axis the curvature
    p + m - 2 z of the row sums, and the bottom (m - p) / (2 curvature),
    at most 3 steps out; both in 1/16 steps (cut towards 0) */
#define R_P 9
#define R_MM 10
#define R_Z 11
#define R_DX 12
static const char p_bowl[] = {LD(6), ADD(7), ADD(8), ST(R_P), LD(0), ADD(1), ADD(2), ST(R_MM),
    LD(3), ADD(4), ADD(5), ST(R_Z), ADD(R_Z), ST(R_Z), LD(R_P), ADD(R_MM), SUB(R_Z), POS, ST(R_Z),
    LD(R_MM), SUB(R_P), DIV(R_Z), K(MUL(K_HALF)), K(MIN(K_3)), NEG, K(MIN(K_3)), NEG, ST(R_DX),
    LD(2), ADD(5), ADD(8), ST(R_P), LD(0), ADD(3), ADD(6), ST(R_MM),
    LD(1), ADD(4), ADD(7), ST(R_Z), ADD(R_Z), ST(R_Z), LD(R_P), ADD(R_MM), SUB(R_Z), POS, ST(R_Z),
    LD(R_MM), SUB(R_P), DIV(R_Z), K(MUL(K_HALF)), K(MIN(K_3)), NEG, K(MIN(K_3)), NEG, K(MUL(K_16)), ST(R_Z),
    LD(R_DX), K(MUL(K_16)), OUT, LD(R_Z), OUT, END};

#define M_CACHE 10
#define M_KNEE 324 // the reflected diode reads 0 below this ("x3" mV)
static char t_l_max, t_c_max;
// The last readings: L, C | 128 on the LC side, SWR (0 = empty), |g|^2
static struct cache {
    char l, key;
    int swr;
    float g2;
} t_cache[M_CACHE];
static char t_cache_next;
/*  Per line (0: C = 0, 1: L = 0): the sums. Fixed once the calculation
    starts (t_frozen); the bowl's 3x3 patch reuses them after */
static union {
    float sum[2][8];
    float g[9];
} t_u;
static char t_n[2], t_frozen;
// Candidate settings from the calculation: L, C, side
static struct cand {
    char l, c, sw;
} t_cand[8];
// The best match, or the best three grid points; T_BEST: the best so far
#define T_BEST 3
static struct top {
    int swr;
    char l, c, sw;
} t_top[4];
#define t_best t_top[T_BEST].swr

static int m_value(char code, char cap) {
    int *val = cap ? e_i_Cap : e_i_Ind, v = 0;
    char i;
    for (i = 0; i < 7; i++, code >>= 1)
        if (code & 1)
            v += val[i];
    return v;
}

/*  The relay code whose total (nH, or pF with cap) is closest to target.
    The codes are walked in Gray order: one relay changes per step */
static char m_nearest(unsigned int t, char cap) {
    int *val = cap ? e_i_Cap : e_i_Ind;
    char b, i, bit, g = 0, best = 0, max = cap ? t_c_max : t_l_max;
    unsigned int v = 0, d, bd = 0xFFFF;
    for (b = 1;; b++) {
        d = v > t ? v - t : t - v;
        if (d < bd && g <= max) {
            bd = d;
            best = g;
        }
        if (b == 128)
            return best;
        for (i = 0, bit = 1; !(b & bit); i++)
            bit <<= 1;
        g ^= bit;
        if (g & bit)
            v += val[i];
        else
            v -= val[i];
    }
}

// A reading on a line into its sums (R_R, R_F set): x the code's nH or pF
static void m_add(char line, char code, int cr) {
    float *sum = t_u.sum[line];
    memcpy(t_v, sum, 32);
    m_in(R_X, m_value(code, line));
    m_calc(cr ? p_sig_read : p_sig_knee);
    m_calc(p_add);
    memcpy(sum, t_v, 32);
    t_n[line]++;
}

/*  Set the relays, one settling wait when L and C change together, and
    measure (again for a new best, keeping the average): g_i_SWR, and
    |g|^2 in t_v[R_G2]. 0 when the tune was cancelled */
static char m_measure(char l, char c, char sw, char again) {
    char i, key = c | sw << 7;
    int cr = 0, cf = 0, first, knee = 0;
    struct cache *e = t_cache;
    for (i = 0; i < M_CACHE; i++, e++)
        if (e->swr && e->l == l && e->key == key) {
            g_i_SWR = e->swr;
            t_v[R_G2] = e->g2;
            return 1;
        }
    if (sw != g_c_SW)
        set_sw(g_c_SW = sw);
    if (l != g_c_ind && c != g_c_cap) {
        first = e_i_ms_Rel_Del;
        e_i_ms_Rel_Del = 0;
        set_ind(g_c_ind = l);
        e_i_ms_Rel_Del = first;
    }
    if (l != g_c_ind)
        set_ind(g_c_ind = l);
    if (c != g_c_cap)
        set_cap(g_c_cap = c);
    for (;;) {
        get_swr();
        if (g_i_SWR == 0)
            return 0;
        cf += correction((int) g_u_fwd * 3);
        cr += correction((int) g_u_rev * 3);
        if (knee) {
            g_i_SWR = (g_i_SWR + first) / 2;
            knee += M_KNEE;
            break;
        }
        knee = M_KNEE;
        first = g_i_SWR;
        if (!again || first >= t_best)
            break;
    }
    if (cf == 0)
        cr = cf = 1; // no forward reading: as a total mismatch
    m_in(R_F, knee);
    m_in(R_R, cr ? cr : knee >> 1);
    m_in(R_CF, cf);
    m_calc(p_rho);
    e = &t_cache[t_cache_next];
    e->l = l;
    e->key = key;
    e->swr = g_i_SWR;
    e->g2 = t_v[R_G2];
    if (++t_cache_next == M_CACHE)
        t_cache_next = 0;
    if (g_i_SWR < t_best) {
        t_best = g_i_SWR;
        t_top[T_BEST].l = l;
        t_top[T_BEST].c = c;
        t_top[T_BEST].sw = sw;
    }
    /*  On a line: into its sums (the bypass into both). A reading still
        in t_cache is not measured again, so not added again either */
    if (!t_frozen) {
        if (c == 0)
            m_add(0, l, cr);
        if (l == 0)
            m_add(1, c, cr);
    }
    return 1;
}

// A line's parabola into the registers; 0 if none (fewer than 3 readings, or no bottom)
static char m_fit(char line) {
    if (t_n[line] < 3)
        return 0;
    memcpy(t_v, t_u.sum[line], 32);
    return m_calc(p_fit);
}

/*  The antenna from a line's parabola (both answers), and the relays of
    an exact match on each side: t_cand, how many. Only calculates: the
    caller measures */
static char m_codes(char line) {
    char root, side, sign, n = 0;
    if (!m_fit(line) || !m_calc(p_roots))
        return 0;
    for (root = 0; root < 2; root++) {
        if (!m_calc(root ? p_minus : p_plus) || !m_calc(p_ant))
            continue;
        if (line)
            m_calc(p_admit);
        if (!m_calc(p_check))
            continue;
        for (side = 0; side < 2; side++) // 1: C at the antenna (LC)
            if (m_calc(side ? p_lc_s : p_cl_s))
                for (sign = 0; sign < 2; sign++) {
                    if (sign)
                        m_calc(p_neg_s);
                    if (m_calc(side ? p_lc : p_cl) && m_calc(p_target)) {
                        t_cand[n].l = m_nearest(t_out[0], 0);
                        t_cand[n].c = m_nearest(t_out[1], 1);
                        t_cand[n++].sw = side;
                    }
                }
    }
    return n;
}

// Keep the best n points in t_top (in no order), the point just measured among them
static void m_top(char n, char l, char c, char sw) {
    struct top *p = t_top, *w = t_top;
    for (; n; n--, p++) // w: an empty place, or the worst
        if (w->swr && (p->swr == 0 || p->swr > w->swr))
            w = p;
    if (w->swr == 0 || g_i_SWR < w->swr) {
        w->swr = g_i_SWR;
        w->l = l;
        w->c = c;
        w->sw = sw;
    }
}

// A bowl step: fixed, or a 1/dv of the value (at least one relay), 2 from 0
static char m_step(char v, char dv, char fixed) {
    if (fixed)
        return fixed;
    if (v == 0)
        return 2;
    v = (v + dv / 2) / dv;
    return v ? v : 1;
}

// v0 moved by out sixteenths of step, kept within 0..max
static int m_move(char v0, int out, char step, char max) {
    int v = v0 + ((out * step + 8) >> 4);
    return v < 0 ? 0 : v > max ? max : v;
}

/*  From t_top[n]: measure a 3x3 patch around the point (steps a quarter of
    L and C, at least one relay; or fixed relays), fit the bowl, measure at
    its bottom; again from the best point, the patch halved when nothing
    was better. From the best with fixed 2, it is the last single steps */
static char m_bowl(char n, char fixed) {
    struct top *p = &t_top[n];
    int lv = p->swr, l, c;
    char ll = p->l, lc = p->c, sw = p->sw;
    char l0, c0, sl, sc, dv = 4, jump, i, k;
    for (jump = 0; jump < 6 + fixed; jump++) {
        l0 = ll;
        c0 = lc;
        sl = m_step(l0, dv, fixed);
        sc = m_step(c0, dv, fixed);
        l = l0 - sl;
        c = c0 - sc - sc;
        for (i = k = 0; i < 10; i++) {
            if (i < 9) { // by rows: L - step, L, L + step
                c += sc;
                if (i == 3 || i == 6) {
                    c = c0 - sc;
                    l += sl;
                }
                if (l < 0 || c < 0 || l > t_l_max || c > t_c_max)
                    continue;
            } else { // the bottom of the bowl
                memcpy(t_v, t_u.g, 36);
                if (k < 9 || !m_calc(p_bowl))
                    break;
                l = m_move(l0, t_out[0], sl, t_l_max);
                c = m_move(c0, t_out[1], sc, t_c_max);
            }
            if (!m_measure((char) l, (char) c, sw, 1))
                return 0;
            if (g_i_SWR < lv) {
                lv = g_i_SWR;
                ll = (char) l;
                lc = (char) c;
            }
            if (i < 9) {
                t_u.g[i] = t_v[R_G2];
                k++;
            }
        }
        if (ll == l0 && lc == c0) {
            if (sl == 1 && sc == 1)
                break;
            dv <<= 1; // nothing better: look closer
            fixed >>= 1;
        }
        if (lv < 105)
            break;
    }
    return 1;
}

static const char tune_grid[] = {0, 2, 5, 12, 30, 70, 127};

// The 7x7 grid on both sides, then the bowl from its best three points
static char m_grid(void) {
    char sw, li, ci, l, c;
    t_frozen = 1;
    memset(t_top, 0, 3 * sizeof t_top[0]);
    for (sw = 2; sw-- > 0;)
        for (li = 0; li < sizeof tune_grid && (l = tune_grid[li]) <= t_l_max; li++)
            for (ci = 0; ci < sizeof tune_grid && (c = tune_grid[ci]) <= t_c_max; ci++) {
                if (!m_measure(l, c, sw, 0))
                    return 0;
                m_top(3, l, c, sw);
            }
    for (li = 0; li < 3; li++)
        if (t_top[li].swr && !m_bowl(li, 0))
            return 0;
    return m_bowl(T_BEST, 2);
}

static const char m_probe[] = {0, 4, 16, 64, 4, 16, 64};

// A reading on a line: L alone (line 0) or C alone (1)
static char m_line(char line, char code, char sw) {
    return m_measure(line ? 0 : code, line ? code : 0, sw, 0);
}

// 0 when cancelled, 1 when done, 2 when the grid has to follow
static char m_search(void) {
    char sw = g_c_SW, i, n, line, root, added;
    struct cand *p;
    for (i = 0; i < 7; i++) // the bypass, three L alone, three C alone
        if (!m_line(i > 3, m_probe[i], sw))
            return 0;
    /*  More readings where each line's parabola has its bottom, and at 3/4
        and 4/3 of it: they tell the most */
    for (root = 0; root < 2; root++) {
        added = 0;
        for (line = 0; line < 2; line++) {
            if (!m_fit(line) || !m_calc(p_vertex))
                continue;
            n = t_n[line];
            for (i = 0; i < 3; i++)
                if (!m_line(line, m_nearest(t_out[i], line), sw))
                    return 0;
            if (t_n[line] != n)
                added = 1;
        }
        if (!added)
            break;
    }
    t_frozen = 1;
    t_top[0].swr = 0;
    for (line = 0; line < 2; line++)
        for (n = m_codes(line), p = t_cand; n; n--, p++) {
            if (!m_measure(p->l, p->c, p->sw, 1))
                return 0;
            m_top(1, p->l, p->c, p->sw);
        }
    if (t_top[0].swr == 0)
        return 2; // no answer from the readings
    if (!m_bowl(0, 0) || !m_bowl(T_BEST, 2))
        return 0;
    return t_best > 130 ? 2 : 1;
}

static void tune_model(void) {
    char i;
    t_l_max = 32 * g_c_L_mult - 1;
    t_c_max = 32 * g_c_C_mult - 1;
    t_best = 10000;
    memset(t_cache, 0, sizeof t_cache);
    memset(&t_u, 0, sizeof t_u);
    t_n[0] = t_n[1] = t_frozen = 0;
    i = m_search();
    if (i == 2)
        i = m_grid(); // a 7x7 grid when that ends above SWR 1.3
    if (i == 0) {
        g_i_SWR = 0;
        return;
    }
    if (g_c_SW != t_top[T_BEST].sw)
        set_sw(g_c_SW = t_top[T_BEST].sw);
    set_ind(g_c_ind = t_top[T_BEST].l);
    set_cap(g_c_cap = t_top[T_BEST].c);
    g_i_SWR = t_best;
}

void tune() {
    CLRWDT();
    //
    g_char_p_cnt = 0;
    g_i_P_max = 0;
    //
    g_b_rready = 0;
    get_swr();
    if (g_i_SWR < 110)
        return;
    atu_reset();
    if (e_c_b_Loss_ind == 0)
        lcd_ind();
    Delay_ms(50);
    get_swr();
    g_i_swr_a = g_i_SWR;
    if (g_i_SWR < 110)
        return;
    if (e_i_tenths_init_max_swr > 110 & g_i_SWR > e_i_tenths_init_max_swr)
        return;
    //
    tune_model();
    if (g_i_SWR == 0)
        atu_reset();
}
