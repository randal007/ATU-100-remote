/*  Cycle benchmark of main.h's calculator (run.sh builds and runs it).
    Each program once on typical register values, timed with Timer1 at
    1:8 into res[], which run.sh reads from gpsim's RAM dump. Then the old
    inline float code of get_pwr and show_pwr, for comparison, and
    m_nearest. */
#include <xc.h>
#include <string.h>
#pragma config FOSC = INTOSC, WDTE = OFF, PLLEN = OFF, LVP = OFF
static int e_i_Ind[7] = {50, 110, 220, 450, 1000, 2200, 4500};
static int e_i_Cap[7] = {10, 22, 47, 100, 220, 470, 1000};
static char t_l_max = 127, t_c_max = 127;
#include "bench_calc.h"
volatile char code;

volatile unsigned int res[24] __at(0x2000);
static char k;

static void t0(void) { TMR1H = 0; TMR1L = 0; TMR1ON = 1; }
static void t1(void) { TMR1ON = 0; res[k++] = TMR1L | (unsigned int) TMR1H << 8; }

static const float fit_in[8] = {522.8346, 118.764, 81.1493, 110.0613, 283.7645, 832.8422, 224.6003, 216.7805};

void main(void) {
    T1CON = 0x30; // Fosc/4, 1:8
    // the measure: p_rho, p_sig_read, p_add
    t_v[R_R] = 3000; t_v[R_F] = 324; t_v[R_CF] = 7000;
    t0(); m_calc(p_rho); t1();
    memcpy(t_v, fit_in, 32); t_v[R_X] = 1000;
    t0(); m_calc(p_sig_read); t1();
    t0(); m_calc(p_add); t1();
    // the fit and the vertex
    memcpy(t_v, fit_in, 32);
    t0(); m_calc(p_fit); t1();
    t_v[R_C1] = -2; t_v[R_C2] = 1.5; t_v[R_C0] = 3;
    t0(); m_calc(p_vertex); t1();
    // the antenna and the matches
    t0(); m_calc(p_roots); t1();
    t0(); m_calc(p_plus); t1();
    t0(); m_calc(p_ant); t1();
    t0(); m_calc(p_admit); t1();
    t_v[R_WW] = 1.2; t_v[R_RE] = 0.5; t_v[R_IM] = 0.3;
    t0(); m_calc(p_check); t1();
    t0(); m_calc(p_lc_s); t1();
    t0(); m_calc(p_lc); t1();
    t0(); m_calc(p_cl_s); t1();
    t0(); m_calc(p_cl); t1();
    t_v[R_WL] = 0.6; t_v[R_WC] = 0.4;
    t0(); m_calc(p_target); t1();
    // the bowl
    for (char i = 0; i < 9; i++) t_v[i] = 0.1 + 0.01 * (i % 3) + 0.02 * (i / 3) + 0.005 * i * i;
    t0(); m_calc(p_bowl); t1();
    // power and loss (every reading / every 100)
    t_v[0] = 3000; t_v[1] = 32; t_v[2] = 5;
    t0(); m_calc(p_power); t1();
    t_v[0] = 150; t_v[1] = 5; t_v[2] = 130; t_v[3] = 100;
    t0(); m_calc(p_loss); t1();
    t0(); t1(); // the timer's own cost
    // the old code: get_pwr's float power, show_pwr's float loss
    {
        volatile long fwd = 3000; volatile char kmult = 32; volatile double d; volatile int out;
        t0();
        d = fwd; d = d * kmult / 1000.0; d = d / 1.414; d = d * d / 5; d = d + 0.5; out = (int) d;
        t1();
        volatile int swr_a = 150, loss = 5, swr = 130, pw = 100; double a, b;
        t0();
        a = 1.0 / ((swr_a / 100.0 + 100.0 / swr_a) * loss / 10.0 * 0.115 + 1.0);
        b = 4.0 / (2.0 + swr / 100.0 + 100.0 / swr);
        if (a >= 1.0) a = 1.0;
        if (b >= 1.0) b = 1.0;
        out = (int)(pw * a * b); out = (int)(a * b * 100);
        t1();
    }
    t0(); code = m_nearest(3333, 0); t1();
    for (;;) ;
}
