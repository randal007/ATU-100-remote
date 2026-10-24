
#include "cross_compiler.h"
#include "uart.h"

// Main.h
// David Fainitski
// ATU-100 project 2016

//
static char g_c_ind = 0, g_c_cap = 0;
static char g_c_SW = 0;
static char g_c_step_cap = 0, g_c_step_ind = 0;
static char g_c_L_mult = 1, g_c_C_mult = 1;
int g_i_SWR, g_i_PWR, g_i_P_max, g_i_swr_a;
static char g_b_rready = 0, g_char_p_cnt = 0;
// A tune given up while waiting for carrier: reset once the search has
// returned (tune_btn_push), not down here, where the call stack is deep
static char g_b_tune_given_up = 0;

static char g_b_Overload = 0;

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
/* tuning search: 0 = N7DDC's original, anything else = log grid search */
static char e_c_tune_mode = 1;

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
void coarse_cap();
void sharp_cap();
void sharp_ind();
void coarse_tune();
void tune(void);
void sub_tune(void);
//

int correction(int input) {
    //
    if (input <= 80)
        return 0;
    if (input <= 171)
        input += 244;
    else if (input <= 328)
        input += 254;
    else if (input <= 582)
        input += 280;
    else if (input <= 820)
        input += 297;
    else if (input <= 1100)
        input += 310;
    else if (input <= 2181)
        input += 430;
    else if (input <= 3322)
        input += 484;
    else if (input <= 4623)
        input += 530;
    else if (input <= 5862)
        input += 648;
    else if (input <= 7146)
        input += 743;
    else if (input <= 8502)
        input += 800;
    else if (input <= 10500)
        input += 840;
    else
        input += 860;
    //
    return input;
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
    double l_doub_pwr;
    CLRWDT();
    //
    l_Forward = get_forward();
    l_Reverse = get_reverse();
    if (e_c_b_D_correction == 1)
        l_doub_pwr = correction((int) (l_Forward * 3));
    else
        l_doub_pwr = l_Forward * 3;
    //
    if (l_Reverse >= l_Forward)
        l_Forward = 999;
    else {
        l_Forward = ((l_Forward + l_Reverse) * 100) / (l_Forward - l_Reverse);
        if (l_Forward > 999)
            l_Forward = 999;
    }
    //
    l_doub_pwr = l_doub_pwr * e_c_K_Mult / 1000.0; // mV to Volts on Input
    l_doub_pwr = l_doub_pwr / 1.414;
    if (e_c_b_P_High == 1)
        l_doub_pwr = l_doub_pwr * l_doub_pwr / 50; // 0 - 1500 ( 1500 Watts)
    else
        l_doub_pwr = l_doub_pwr * l_doub_pwr / 5; // 0 - 1510 (151.0 Watts)
    l_doub_pwr = l_doub_pwr + 0.5; // rounding
    //
    g_i_PWR = (int) (l_doub_pwr);
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

void coarse_cap() {
    char l_coarse_cap_step = 3;
    char l_coarse_cap_count;
    int l_coarse_cap_min_swr;

    g_c_cap = 0;
    set_cap(g_c_cap);
    g_c_step_cap = l_coarse_cap_step;
    get_swr();
    if (g_i_SWR == 0)
        return;
    l_coarse_cap_min_swr = g_i_SWR + g_i_SWR / 20;
    for (l_coarse_cap_count = l_coarse_cap_step; l_coarse_cap_count <= 31;) {
        set_cap(l_coarse_cap_count * g_c_C_mult);
        get_swr();
        if (g_i_SWR == 0)
            return;
        if (g_i_SWR < l_coarse_cap_min_swr) {
            l_coarse_cap_min_swr = g_i_SWR + g_i_SWR / 20;
            g_c_cap = l_coarse_cap_count * g_c_C_mult;
            g_c_step_cap = l_coarse_cap_step;
            if (g_i_SWR < 120)
                break;
            l_coarse_cap_count += l_coarse_cap_step;
            if (e_c_b_C_linear == 0 & l_coarse_cap_count == 9)
                l_coarse_cap_count = 8;
            else if (e_c_b_C_linear == 0 & l_coarse_cap_count == 17) {
                l_coarse_cap_count = 16;
                l_coarse_cap_step = 4;
            }
        } else
            break;
    }
    set_cap(g_c_cap);
    return;
}

void coarse_tune() {
    char l_coarse_tune_step = 3;
    char l_coarse_tune_count;
    char l_coarse_tune_mem_cap, l_coarse_tune_mem_step_cap;
    int l_coarse_tune_min_swr;

    l_coarse_tune_mem_cap = 0;
    g_c_step_ind = l_coarse_tune_step;
    l_coarse_tune_mem_step_cap = 3;
    l_coarse_tune_min_swr = g_i_SWR + g_i_SWR / 20;
    for (l_coarse_tune_count = 0; l_coarse_tune_count <= 31;) {
        set_ind(l_coarse_tune_count * g_c_L_mult);
        coarse_cap();
        get_swr();
        if (g_i_SWR == 0)
            return;
        if (g_i_SWR < l_coarse_tune_min_swr) {
            l_coarse_tune_min_swr = g_i_SWR + g_i_SWR / 20;
            g_c_ind = l_coarse_tune_count * g_c_L_mult;
            l_coarse_tune_mem_cap = g_c_cap;
            g_c_step_ind = l_coarse_tune_step;
            l_coarse_tune_mem_step_cap = g_c_step_cap;
            if (g_i_SWR < 120)
                break;
            l_coarse_tune_count += l_coarse_tune_step;
            if (e_c_b_L_linear == 0 & l_coarse_tune_count == 9)
                l_coarse_tune_count = 8;
            else if (e_c_b_L_linear == 0 & l_coarse_tune_count == 17) {
                l_coarse_tune_count = 16;
                l_coarse_tune_step = 4;
            }
        } else
            break;
    }
    g_c_cap = l_coarse_tune_mem_cap;
    set_ind(g_c_ind);
    set_cap(g_c_cap);
    g_c_step_cap = l_coarse_tune_mem_step_cap;
    Delay_ms(10);
    return;
}

void sharp_cap() {
    char l_sharp_cap_range, l_sharp_cap_count, l_sharp_cap_max_range, l_sharp_cap_min_range;
    int l_sharp_cap_min_SWR;
    l_sharp_cap_range = g_c_step_cap * g_c_C_mult;
    //
    l_sharp_cap_max_range = g_c_cap + l_sharp_cap_range;
    if (l_sharp_cap_max_range > 32 * g_c_C_mult - 1)
        l_sharp_cap_max_range = 32 * g_c_C_mult - 1;
    if (g_c_cap > l_sharp_cap_range)
        l_sharp_cap_min_range = g_c_cap - l_sharp_cap_range;
    else
        l_sharp_cap_min_range = 0;
    g_c_cap = l_sharp_cap_min_range;
    set_cap(g_c_cap);
    get_swr();
    if (g_i_SWR == 0)
        return;
    l_sharp_cap_min_SWR = g_i_SWR;
    for (l_sharp_cap_count = l_sharp_cap_min_range + g_c_C_mult; l_sharp_cap_count <= l_sharp_cap_max_range; l_sharp_cap_count += g_c_C_mult) {
        set_cap(l_sharp_cap_count);
        get_swr();
        if (g_i_SWR == 0)
            return;
        if (g_i_SWR >= l_sharp_cap_min_SWR) {
            Delay_ms(10);
            get_swr();
        }
        if (g_i_SWR >= l_sharp_cap_min_SWR) {
            Delay_ms(10);
            get_swr();
        }
        if (g_i_SWR < l_sharp_cap_min_SWR) {
            l_sharp_cap_min_SWR = g_i_SWR;
            g_c_cap = l_sharp_cap_count;
            if (g_i_SWR < 120)
                break;
        } else
            break;
    }
    set_cap(g_c_cap);
    return;
}

void sharp_ind() {
    char l_sharp_ind_range, l_sharp_ind_count, l_sharp_ind_max_range, l_sharp_ind_min_range;
    int l_sharp_ind_min_SWR;
    l_sharp_ind_range = g_c_step_ind * g_c_L_mult;
    //
    l_sharp_ind_max_range = g_c_ind + l_sharp_ind_range;
    if (l_sharp_ind_max_range > 32 * g_c_L_mult - 1)
        l_sharp_ind_max_range = 32 * g_c_L_mult - 1;
    if (g_c_ind > l_sharp_ind_range)
        l_sharp_ind_min_range = g_c_ind - l_sharp_ind_range;
    else
        l_sharp_ind_min_range = 0;
    g_c_ind = l_sharp_ind_min_range;
    set_ind(g_c_ind);
    get_swr();
    if (g_i_SWR == 0)
        return;
    l_sharp_ind_min_SWR = g_i_SWR;
    for (l_sharp_ind_count = l_sharp_ind_min_range + g_c_L_mult; l_sharp_ind_count <= l_sharp_ind_max_range; l_sharp_ind_count += g_c_L_mult) {
        set_ind(l_sharp_ind_count);
        get_swr();
        if (g_i_SWR == 0)
            return;
        if (g_i_SWR >= l_sharp_ind_min_SWR) {
            Delay_ms(10);
            get_swr();
        }
        if (g_i_SWR >= l_sharp_ind_min_SWR) {
            Delay_ms(10);
            get_swr();
        }
        if (g_i_SWR < l_sharp_ind_min_SWR) {
            l_sharp_ind_min_SWR = g_i_SWR;
            g_c_ind = l_sharp_ind_count;
            if (g_i_SWR < 120)
                break;
        } else
            break;
    }
    set_ind(g_c_ind);
    return;
}

void sub_tune() {
    int l_int_swr_mem, l_int_ind_mem, l_int_cap_mem;
    //
    l_int_swr_mem = g_i_SWR;
    coarse_tune();
    if (g_i_SWR == 0) {
        atu_reset();
        return;
    }
    get_swr();
    if (g_i_SWR < 120)
        return;
    sharp_ind();
    if (g_i_SWR == 0) {
        atu_reset();
        return;
    }
    get_swr();
    if (g_i_SWR < 120)
        return;
    sharp_cap();
    if (g_i_SWR == 0) {
        atu_reset();
        return;
    }
    get_swr();
    if (g_i_SWR < 120)
        return;
    //
    if (g_i_SWR < 200 & g_i_SWR < l_int_swr_mem & (l_int_swr_mem - g_i_SWR) > 100)
        return;
    l_int_swr_mem = g_i_SWR;
    l_int_ind_mem = g_c_ind;
    l_int_cap_mem = g_c_cap;
    //
    if (g_c_SW == 1)
        g_c_SW = 0;
    else
        g_c_SW = 1;
    atu_reset();
    set_sw(g_c_SW);
    Delay_ms(50);
    get_swr();
    if (g_i_SWR < 120)
        return;
    //
    coarse_tune();
    if (g_i_SWR == 0) {
        atu_reset();
        return;
    }
    get_swr();
    if (g_i_SWR < 120)
        return;
    sharp_ind();
    if (g_i_SWR == 0) {
        atu_reset();
        return;
    }
    get_swr();
    if (g_i_SWR < 120)
        return;
    sharp_cap();
    if (g_i_SWR == 0) {
        atu_reset();
        return;
    }
    get_swr();
    if (g_i_SWR < 120)
        return;
    //
    if (g_i_SWR > l_int_swr_mem) {
        if (g_c_SW == 1)
            g_c_SW = 0;
        else
            g_c_SW = 1;
        set_sw(g_c_SW);
        g_c_ind = (char) (l_int_ind_mem);
        g_c_cap = (char) (l_int_cap_mem);
        set_ind(g_c_ind);
        set_cap(g_c_cap);
        g_i_SWR = l_int_swr_mem;
    }
    //
    CLRWDT();
    return;
}


/*  Search over a logarithmic grid, then refine L and C together.
    The relay values double from bit to bit, so the relay number is close
    to linear in uH and pF; a log spaced grid gives small values (high
    bands) the same relative resolution as large ones (low bands). */
static const char tune_grid[] = {0, 1, 2, 3, 4, 6, 8, 11, 16, 22, 32, 45, 64, 90, 127};
static int t_best;
static char t_best_l, t_best_c, t_best_sw;

// Measure at (l, c); 0 when the tune was cancelled
static char t_try(char l, char c) {
    if (l != g_c_ind) set_ind(g_c_ind = l);
    if (c != g_c_cap) set_cap(g_c_cap = c);
    get_swr();
    if (g_i_SWR == 0)
        return 0;
    if (g_i_SWR < t_best) { // a new best: measure again, keep the average
        int first = g_i_SWR;
        get_swr();
        if (g_i_SWR == 0)
            return 0;
        g_i_SWR = (g_i_SWR + first) / 2;
    }
    if (g_i_SWR < t_best) {
        t_best = g_i_SWR;
        t_best_l = l;
        t_best_c = c;
        t_best_sw = g_c_SW;
    }
    return 1;
}

static void tune_log(void) {
    char sw, li, ci, rises, row_rises, step, moved;
    char l_max = 32 * g_c_L_mult - 1, c_max = 32 * g_c_C_mult - 1;
    int row, last_row;

    t_best = 10000;
    for (sw = 0; sw < 2; sw++) {
        set_sw(g_c_SW = !g_c_SW);
        last_row = 10000;
        row_rises = 0;
        for (li = 0; li < sizeof tune_grid && tune_grid[li] <= l_max; li++) {
            row = 10000;
            rises = 0;
            for (ci = 0; ci < sizeof tune_grid && tune_grid[ci] <= c_max; ci++) {
                if (!t_try(tune_grid[li], tune_grid[ci]))
                    return;
                if (g_i_SWR < row) {
                    row = g_i_SWR;
                    rises = 0;
                } else if (g_i_SWR > row + row / 20 && ++rises == 2)
                    break; // past the dip along C (9.99 everywhere is no dip)
            }
            if (t_best < 110)
                break;
            // Past the dip along L too: two rows in a row worse than the last
            if (row > last_row + last_row / 20 && t_best < 999) {
                if (++row_rises == 2)
                    break;
            } else
                row_rises = 0;
            last_row = row;
        }
        if (t_best <= 130)
            break; // good enough to refine without trying C on the other side
    }
    // Refine around the best point, moving L and C together
    if (g_c_SW != t_best_sw)
        set_sw(g_c_SW = t_best_sw);
    for (step = 4; step;) {
        signed char dl, dc;
        moved = 0;
        for (dl = -1; dl <= 1; dl++)
            for (dc = -1; dc <= 1; dc++) {
                int l = t_best_l + dl * step, c = t_best_c + dc * step;
                char bl = t_best_l, bc = t_best_c;
                if ((dl | dc) == 0 || l < 0 || c < 0 || l > l_max || c > c_max)
                    continue;
                if (!t_try((char)l, (char)c))
                    return;
                if (t_best_l != bl || t_best_c != bc)
                    moved = 1;
            }
        if (!moved)
            step >>= 1;
        if (t_best < 105)
            break;
    }
    set_ind(g_c_ind = t_best_l);
    set_cap(g_c_cap = t_best_c);
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
    if (e_c_tune_mode != 0) {
        tune_log();
        if (g_i_SWR == 0)
            atu_reset();
        return;
    }
    sub_tune();
    if (g_i_SWR == 0) {
        atu_reset();
        return;
    }
    if (g_i_SWR < 120)
        return;
    if (e_c_num_C_q == 5 & e_c_num_L_q == 5)
        return;

    if (e_c_num_L_q > 5) {
        g_c_step_ind = g_c_L_mult;
        g_c_L_mult = 1;
        sharp_ind();
    }
    if (g_i_SWR < 120)
        goto restore; // not return: g_c_L_mult must go back (N7DDC returned here)
    if (e_c_num_C_q > 5) {
        g_c_step_cap = g_c_C_mult; // = g_c_C_mult
        g_c_C_mult = 1;
        sharp_cap();
    }
restore:
    if (e_c_num_L_q == 5)
        g_c_L_mult = 1;
    else if (e_c_num_L_q == 6)
        g_c_L_mult = 2;
    else if (e_c_num_L_q == 7)
        g_c_L_mult = 4;
    if (e_c_num_C_q == 5)
        g_c_C_mult = 1;
    else if (e_c_num_C_q == 6)
        g_c_C_mult = 2;
    else if (e_c_num_C_q == 7)
        g_c_C_mult = 4;
    CLRWDT();
    return;
}
