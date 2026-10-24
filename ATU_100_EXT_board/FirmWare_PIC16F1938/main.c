//   ATU-100 project
//   David Fainitski
//   2016
//
//   Display, all three buttons and serial remote control together:
//   WA1RCT's XC8 port of N7DDC firmware 3.2 with the JSON serial protocol
//   from ukoda/PP5OO. Serial runs on the Tx_req pins 9 and 10, see README.md.

#include "cross_compiler.h"
#include "oled_control.h"
#include "uart.h"
#include "json.h"
#include <string.h>
#include "main.h"

/*  a few constants */

#define DEFAULT_INITIAL_OLD_VALUE  10000

// Sent at start-up as "Build"; raise the revision with each release
#define FW_REVISION "1.1.1" // 1.1: the model tuning search
#define FW_BUILD    "VE7NHW display+serial " FW_REVISION ", " __DATE__

// Variables
int g_i_SWR_fixed_old = 0;
char g_work_str[7], g_work_str_2[7];
int g_i_Power = 0, g_i_Power_old = DEFAULT_INITIAL_OLD_VALUE, g_i_Power_report = 0;
int g_i_SWR_old = DEFAULT_INITIAL_OLD_VALUE;
int g_i_SWR_fixed = 0;
int g_i_Efficency = 0;
char e_c_ledtype, g_b_Soft_tune = 0;
char g_b_Auto_mode = 0;

char g_b_Bypas_mode = 0;
char g_c_cap_mem = 0, g_c_ind_mem = 0, g_c_SW_mem = 0, g_c_Auto_mem = 0;

char g_b_Restart = 0;
char g_b_Test_mode = 0;
char g_b_lcd_prep_short = 0;
char g_b_L = 1, g_b_but = 0;

int g_i_dysp_delay_counter = 0;
int c_int_dysp_delay_time = 0;

char g_b_Loss_mode = 0;
char g_b_display_onoff = 1;
char g_b_tune_btn_released;

bool new_state = false; // send a status message
int g_i_Power_tx = 0;    // transmitter power, as shown after PWR=
int g_i_loss_swr_old = 0; // SWR the antenna power was last worked out for
int e_i_swr_alarm;       // HighSWR event above this, SWR * 100; 0 = off
char g_b_swr_alarm = 0;  // HighSWR sent, waiting for the SWR to come down
int e_i_power_alarm;     // HighPower event above this, tenths of W; 0 = off
char g_b_power_alarm = 0; // HighPower sent, waiting for the power to come down

/*  initial eeprom values*/
__eeprom unsigned char initial_eeprom[256] = {
    0x78,0x05,0x00,0x15,0x13,0x05,0x00,0x00,0x02,0x00,0x07,0x00,0x07,0x00,0x01,0x00,
    0x00,0x50,0x01,0x10,0x02,0x20,0x04,0x50,0x10,0x00,0x22,0x00,0x45,0x00,0xff,0xff,
    0x00,0x10,0x00,0x22,0x00,0x47,0x01,0x00,0x02,0x20,0x04,0x70,0x10,0x00,0xff,0xff,
    0x00,0x10,0x00,0x01,0x12,0x00,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
    0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0x00,0x00,0x01,0x00,0x00,
};

void serial_proc(void);
void send_state(void);
static void start_tune(void);
// A tune asked for by the button or over serial: started from the main
// loop, not from inside button_proc or serial_proc, which would add levels
// to the deepest call chain (the PIC has a 16-level hardware stack)
static char g_b_tune_asked = 0;
static void quick_swr(void);

//***************** Display helpers *****************

static char big_oled(void) // 128*64 OLED
{
   return e_c_ledtype == 4 | e_c_ledtype == 5;
}

// Write at (r64, c64) on a 128*64 OLED, or at (r16, c16) on a 1602 LCD or 128*32 OLED
static void disp(char r64, char c64, char r16, char c16, char *str, char leng)
{
   if (big_oled())
      led_wr_str(r64, c64, str, leng);
   else if (e_c_ledtype != 0)
      led_wr_str(r16, c16, str, leng);
}

// The mark after PWR: "." auto mode, "_" bypass
static void show_mode(void)
{
   char *mark = " ";
   if (g_b_Auto_mode & !g_b_Bypas_mode)
      mark = ".";
   else if ((!g_b_Auto_mode) & g_b_Bypas_mode)
      mark = "_";
   disp(0, 16 + 8 * 12, 0, 8, mark, 1);
   new_state = true;
}

static void show_swr_text(char *str)
{
   disp(2, 16, 1, 0, str, 8);
}

// Two-colour LED: green below SWR 1.5, orange below 2.5, red above. 0 = off
// The LED shares its pins with the display's I2C bus.
static void led_swr(int swr)
{
   if (e_c_ledtype != 0)
      return;
   GREEN_LED = (swr != 0 & swr <= 250) ? LED_ON : LED_OFF;
   RED_LED = (swr > 150) ? LED_ON : LED_OFF;
}

// Turn the display back on after the off timer. Returns 1 if it was off,
// in which case a button press only wakes it.
static char wake(void)
{
   g_i_dysp_delay_counter = c_int_dysp_delay_time;
   if (g_b_display_onoff)
      return 0;
   if (e_c_b_Relay_off == 1)
   {
      set_ind(g_c_ind);
      set_cap(g_c_cap);
      set_sw(g_c_SW);
   }
   g_b_display_onoff = 1;
   dysp_on();
   return 1;
}

static char any_button(void)
{
   return (PORTB_AUTO_BUTTON == BUTTON_PRESSED) | (PORTB_BYPASS_BUTTON == BUTTON_PRESSED) |
          ((PORTB_TUNE_BUTTON == BUTTON_PRESSED) & g_b_tune_btn_released);
}

// Fast return from measuring when a button is held down
static char button_abort(void)
{
   if (any_button())
   {
      button_delay();
      if (g_b_but == 1)
      {
         g_b_but = 0;
         return 1;
      }
   }
   return 0;
}

//***************** Main *****************

void main()
{
   if (STATUSbits.nTO == 0)
      g_b_Restart = 1;
   pic_init();
   uart_init();
   uart_putc('\n'); // ends a line the reset may have cut off
   if (g_b_Restart)
      json_event("Restart"); // the watchdog reset the tuner
   //
   Delay_ms(300);
   CLRWDT();
   cells_init();
   Soft_I2C_Init();
   g_i_dysp_delay_counter = c_int_dysp_delay_time;
   //
   Delay_ms(300);
   CLRWDT();

   //
   /*  test mode?   enter step by step adjustments */
   if (PORTB_AUTO_BUTTON == BUTTON_PRESSED & PORTB_BYPASS_BUTTON == BUTTON_PRESSED)
   { // g_b_Test_mode mode
      g_b_Test_mode = 1;
      g_b_Auto_mode = 0;
   }

   Delay_ms(1500);
   CLRWDT();

   led_init();

   /*   if FAST TEST mode, then turn on all relays, and loop here forever */
   if ((PORTB_AUTO_BUTTON == BUTTON_PRESSED) &&
           (PORTB_BYPASS_BUTTON == BUTTON_PRESSED) &&
           (PORTB_TUNE_BUTTON == BUTTON_PRESSED))
   { // Fast g_b_Test_mode mode (loop)
      disp(0, 12, 0, 3, "FAST TEST", 9);
      set_cap(255);
      set_ind(e_c_b_L_invert ? 0 : 255);
      set_sw(1);
      while (1)
         Delay_ms(500);
   }
   /*   end of FAST TEST code */
   CLRWDT();
   //
   /*  feeder loss mode */
   if (Button(&PORTB, TUNE_BUTTON, 100, BUTTON_PRESSED))
   { //  Fider loss input
      if (big_oled())
      {
         led_wr_str(0, 6, "Fider Loss", 10); // 128*64
         led_wr_str(2, 6, "input", 5);
         led_wr_str(4, 6 + 3 * 12, "dB", 2);
      }
      else if (e_c_ledtype != 0)
      {
         led_wr_str(0, 0, "Fider Loss input", 16); // 1602 | 128*32
         led_wr_str(1, 3, "dB", 2);
      }
      show_loss();
      //
      while (1)
      {
         CLRWDT();
         if (Button(&PORTB, BYPASS_BUTTON, 50, BUTTON_PRESSED) & (e_c_tenths_Fid_loss < 99))
            e_c_tenths_Fid_loss++;
         else if (Button(&PORTB, AUTO_BUTTON, 50, BUTTON_PRESSED) & (e_c_tenths_Fid_loss > 0))
            e_c_tenths_Fid_loss--;
         else
            continue;
         show_loss();
         eeprom_write(EEPROM_FEEDER_LOSS, Dec2Bcd(e_c_tenths_Fid_loss));
         while (Button(&PORTB, BYPASS_BUTTON, 50, BUTTON_PRESSED) | Button(&PORTB, AUTO_BUTTON, 50, BUTTON_PRESSED))
            CLRWDT();
      } // while
   }    //  Fider loss input
   /*  end of feeder input loss code */
   //
   if (g_b_Test_mode == 0)
   {
      g_c_cap = eeprom_read(EEPROM_LAST_CAP);
      g_c_ind = eeprom_read(EEPROM_LAST_IND);
      g_c_SW = eeprom_read(EEPROM_LAST_SW);
      g_i_swr_a = eeprom_read(EEPROM_LAST_SWR_H) * 256;
      g_i_swr_a += eeprom_read(EEPROM_LAST_SWR_L);
      set_ind(g_c_ind);
      set_cap(g_c_cap);
      set_sw(g_c_SW);
      if (g_b_Restart == 1)
         g_b_lcd_prep_short = 1;
      lcd_prep();
   }
   else
      Test_init();

   lcd_ind();
   g_b_tune_btn_released = 1;

   //*******************************

   while (1)
   {
      CLRWDT();
      lcd_pwr();
      //
      if (g_b_Test_mode == 0)
         button_proc();
      else
         button_proc_test();
      serial_proc();
      if (g_b_tune_asked)
      {
         g_b_tune_asked = 0;
         start_tune();
      }
      send_state();
      //
      if (g_i_dysp_delay_counter != 0)
         g_i_dysp_delay_counter--;
      else if (g_b_Test_mode == 0 & e_c_b_Dysp_delay != 0 & g_b_display_onoff == 1)
      {
         if (e_c_b_Relay_off)
         {
            set_ind(0);
            set_cap(0);
            set_sw(0);
         }
         g_b_display_onoff = 0;
         dysp_off();
      }

      // next While code
   }
}

//***************** Actions shared by buttons and serial *****************

static void toggle_auto_mode(void)
{
   g_b_Auto_mode = !g_b_Auto_mode;
   eeprom_write(EEPROM_AUTOMATIC_MODE, g_b_Auto_mode);
   show_mode();
}

static void toggle_bypass_mode(void)
{
   if (g_b_Bypas_mode == 0)
   {
      g_b_Bypas_mode = 1;
      g_c_cap_mem = g_c_cap;
      g_c_ind_mem = g_c_ind;
      g_c_SW_mem = g_c_SW;
      g_c_cap = 0;
      g_c_ind = e_c_b_L_invert ? 255 : 0;
      g_c_SW = 1;
      g_c_Auto_mem = g_b_Auto_mode;
      g_b_Auto_mode = 0;
   }
   else
   {
      g_b_Bypas_mode = 0;
      g_c_cap = g_c_cap_mem;
      g_c_ind = g_c_ind_mem;
      g_c_SW = g_c_SW_mem;
      g_b_Auto_mode = g_c_Auto_mem;
   }
   set_ind(g_c_ind);
   set_cap(g_c_cap);
   set_sw(g_c_SW);
   quick_swr();
   if (g_b_Loss_mode == 0)
      lcd_ind();
   show_mode();
}

static void start_reset(void)
{
   show_reset();
   g_b_Bypas_mode = 0;
   show_mode();
}

static void start_tune(void)
{
   tune_btn_push();
   g_b_Bypas_mode = 0;
   g_b_Soft_tune = 0;
   show_mode();
}

//***************** Buttons *****************

void button_proc_test(void)
{
   if (Button(&PORTB, TUNE_BUTTON, 50, BUTTON_PRESSED))
   { // Tune btn
      Delay_ms(250);
      CLRWDT();
      if (PORTB_TUNE_BUTTON == BUTTON_RELEASED)
      { // short press button
         g_c_SW = !g_c_SW;
         set_sw(g_c_SW);
         lcd_ind();
      }
      else
      { // long press button
         g_b_L = !g_b_L;
         disp(0, 16 + 12 * 8, 0, 8, g_b_L ? "l" : "c", 1);
      }
      while (Button(&PORTB, TUNE_BUTTON, 50, BUTTON_PRESSED))
      {
         lcd_pwr();
         CLRWDT();
      }
   } // END Tune btn
   //
   if (Button(&PORTB, BYPASS_BUTTON, 50, BUTTON_PRESSED))
   { // BYP button: step up
      while (PORTB_BYPASS_BUTTON == BUTTON_PRESSED)
      {
         if (g_b_L & (g_c_ind < 32 * g_c_L_mult - 1))
            set_ind(++g_c_ind);
         else if (!g_b_L & (g_c_cap < 32 * g_c_C_mult - 1))
            set_cap(++g_c_cap);
         lcd_ind();
         lcd_pwr();
         Delay_ms(30);
      }
   } // end of BYP button
   //
   if (Button(&PORTB, AUTO_BUTTON, 50, BUTTON_PRESSED) & (g_b_Bypas_mode == 0))
   { // AUTO button: step down
      while (PORTB_AUTO_BUTTON == BUTTON_PRESSED)
      {
         if (g_b_L & (g_c_ind > 0))
            set_ind(--g_c_ind);
         else if (!g_b_L & (g_c_cap > 0))
            set_cap(--g_c_cap);
         lcd_ind();
         lcd_pwr();
         Delay_ms(30);
      }
   }
   return;
}

void button_proc(void)
{
   if ((g_b_tune_btn_released == 0) && Button(&PORTB, TUNE_BUTTON, 50, BUTTON_RELEASED))
      g_b_tune_btn_released = 1;
   if ((g_b_tune_btn_released == 0) && Button(&PORTB, TUNE_BUTTON, 50, BUTTON_PRESSED))
      g_i_dysp_delay_counter = c_int_dysp_delay_time;
   if ((Button(&PORTB, TUNE_BUTTON, 50, BUTTON_PRESSED) & g_b_tune_btn_released) | g_b_Soft_tune)
   {
      if (wake())
         return;
      Delay_ms(250);
      CLRWDT();
      if (g_b_Soft_tune == 0 & PORTB_TUNE_BUTTON == BUTTON_RELEASED)
         start_reset(); // short press button
      else
      { // long press button: the main loop starts it, as for serial
         g_b_tune_asked = 1;
         g_b_tune_btn_released = 0;
      }
   }
   //
   if (Button(&PORTB, BYPASS_BUTTON, 50, BUTTON_PRESSED))
   { // BYP button
      if (wake())
         return;
      toggle_bypass_mode();
      while (Button(&PORTB, BYPASS_BUTTON, 50, BUTTON_PRESSED))
         lcd_pwr();
   }
   //
   if (Button(&PORTB, AUTO_BUTTON, 50, BUTTON_PRESSED) & (g_b_Bypas_mode == 0))
   { // g_b_Auto_mode button
      if (wake())
         return;
      toggle_auto_mode();
      while (Button(&PORTB, AUTO_BUTTON, 50, BUTTON_PRESSED))
         lcd_pwr();
   }
   return;
}

void button_delay()
{
   if ((Button(&PORTB, TUNE_BUTTON, 25, BUTTON_PRESSED)) | (Button(&PORTB, AUTO_BUTTON, 25, BUTTON_PRESSED)) | (Button(&PORTB, BYPASS_BUTTON, 25, BUTTON_PRESSED)))
   {
      g_b_but = 1;
   }
   return;
}

//***************** Serial *****************

// Inductance in nH, or capacitance in pF, of the relays that are on
static int relay_sum(char relays, int *values)
{
   int sum = 0;
   char i;
   for (i = 0; i < 7; i++, relays >>= 1)
      if (relays & 1)
         sum += values[i];
   return sum;
}

static void send_error(void)
{
   json_event("Error");
}

void send_state(void)
{
   if (!new_state)
      return;
   new_state = false;
   json_start();
   json_bool("Auto", g_b_Auto_mode);
   json_bool("Bypass", g_b_Bypas_mode);
   json_int("Power", g_i_Power_tx, e_c_b_P_High == 0);
   if (g_b_Loss_mode)
   {
      json_int("Antenna", g_i_Power_report, e_c_b_P_High == 0);
      json_int("Efficiency", g_i_Efficency, 0);
   }
   json_int("SWR", g_i_SWR_fixed, 2);
   json_str("Order", (g_c_SW == 1) ? "LC" : "CL");
   json_int("Capacitance", relay_sum(g_c_cap, e_i_Cap), 0);
   json_int("Inductance", relay_sum(g_c_ind, e_i_Ind), 0);
   // the relays themselves, as RelayI and RelayC take them: the totals do
   // not tell them apart with measured or linear values
   json_int("RelayI", g_c_ind & 0x7f, 0);
   json_int("RelayC", (g_c_cap & 0x7f) | (g_c_SW == 1 ? 0x80 : 0), 0);
   json_end();
}

static void send_eeprom(uint8_t address)
{
   char addstr[3], datastr[3];
   addstr[0] = "0123456789abcdef"[address >> 4];
   addstr[1] = "0123456789abcdef"[address & 15];
   addstr[2] = 0;
   address = eeprom_read(address);
   datastr[0] = "0123456789abcdef"[address >> 4];
   datastr[1] = "0123456789abcdef"[address & 15];
   datastr[2] = 0;
   json_str(addstr, datastr);
}

static void dump_eeprom(void)
{
   uint8_t address = 0;
   json_start();
   do
   {
      CLRWDT();
      send_eeprom(address);
   } while (++address);
   json_end();
}

static void get_eeprom(uint8_t address)
{
   json_start();
   send_eeprom(address);
   json_end();
}

// Detector voltages in mV, before the diode correction
static void send_raw(void)
{
   unsigned int fwd = get_forward(), rev = get_reverse();
   json_start();
   json_int("Fwd", fwd * 3, 0); // the ADC sees a third, through 68k/34k
   json_int("Rev", rev * 3, 0);
   json_end();
}

// Settings written over serial take effect at once
static void apply_settings(uint8_t cell)
{
   char auto_mode = g_b_Auto_mode;
   cells_init();
   if (cell != EEPROM_AUTOMATIC_MODE || g_b_Bypas_mode)
      g_b_Auto_mode = auto_mode; // bypass keeps auto off until it ends
   if (cell <= EEPROM_DISPLAY_TYPE || cell == EEPROM_DISPLAY_OFFSET_DOWN || cell == EEPROM_DISPLAY_OFFSET_LEFT)
   { // display address, type or position: start the display again
      Soft_I2C_Init();
      led_init();
      g_b_lcd_prep_short = 1;
      lcd_prep();
      g_b_Loss_mode = 0; // lcd_prep drew L and C: ANT and EFF are drawn again
   }
   g_i_dysp_delay_counter = c_int_dysp_delay_time; // the display-off timer starts again
   show_mode();
}

// Single letters typed by hand (t, r, b, a, s) count only when they are the
// whole line. Letters left over from a damaged JSON message (one that lost
// its '{', so "Bypass": true} reads as B, a, t...) never act as button
// presses, because other characters share their line. Fed every character
// received; gives the letter when its line ends, else 0.
static char line_letter(char c)
{
   static char letter = 0, others = 0;
   if (c == '\n' || c == '\r')
   {
      char l = others ? 0 : letter;
      letter = 0;
      others = 0;
      return l;
   }
   if (c == ' ' || c == '\t')
      return 0;
   if (letter || others)
      others = 1;
   else
      letter = c;
   return 0;
}

// A letter alone on its line acts like the buttons
static void serial_char(char c)
{
   c |= 0x20; // lower case
   if (c == 'r' | c == 't' | c == 'b' | c == 'a')
      wake();
   switch (c)
   {
   case 'r':
      start_reset();
      break;
   case 't':
      g_b_tune_asked = 1;
      break;
   case 'b':
      toggle_bypass_mode();
      break;
   case 'a':
      if (g_b_Bypas_mode == 0)
         toggle_auto_mode();
      break;
   case 's':
      new_state = true;
      break;
   }
}

void serial_proc(void)
{
   static int cell = -1; // from this message's "Set", for its "Value"
   char c;

   while ((c = uart_getc()) != 0)
   {
      CLRWDT();
      char letter = line_letter(c);
      if (c == '{')
         cell = -1; // a new message: a Value needs its own Set
      json_rx_result_t result = json_rx_process(c);
      if (letter && result == JRR_NOT_JSON)
         serial_char(letter);
      if (result == JRR_NO_DATA | result == JRR_END)
         continue;

      switch (result)
      {
      case JRR_NOT_JSON:
         break; // letters: above, when their line ends

      case JRR_BOOL:
         if (strcmp(json_rx_name, "Auto") == 0)
         {
            if (g_b_Bypas_mode)
               send_error();
            else if (json_rx_bool != g_b_Auto_mode)
               toggle_auto_mode();
         }
         else if (strcmp(json_rx_name, "Bypass") == 0)
         {
            if (json_rx_bool != g_b_Bypas_mode)
               toggle_bypass_mode();
         }
         else if (strcmp(json_rx_name, "Reset") == 0)
            start_reset();
         else if (strcmp(json_rx_name, "Tune") == 0)
            g_b_tune_asked = 1;
         else if (strcmp(json_rx_name, "Raw") == 0)
            send_raw();
         else if (strcmp(json_rx_name, "Status") != 0)
            send_error();
         if (strcmp(json_rx_name, "Tune") != 0)
            new_state = true; // a tune sends its status after Tuned
         if (strcmp(json_rx_name, "Status") != 0 && strcmp(json_rx_name, "Raw") != 0)
            wake(); // like a button press
         break;

      case JRR_INT:
         if (json_rx_int < 0 || json_rx_int > 255)
         { // every number taken is a cell, a byte or relay bits
            send_error();
            cell = -1;
         }
         else if (strcmp(json_rx_name, "Get") == 0)
            get_eeprom((uint8_t)json_rx_int);
         else if (strcmp(json_rx_name, "Set") == 0)
            cell = json_rx_int;
         else if (strcmp(json_rx_name, "Value") == 0)
         {
            // only with a Set earlier in the same message: a damaged one
            // must not write the cell a previous message named
            if (cell < 0)
               send_error();
            else
            {
               eeprom_write((uint8_t)cell, (unsigned char)json_rx_int);
               get_eeprom((uint8_t)cell);
               apply_settings((uint8_t)cell);
            }
            cell = -1;
         }
         else if (strcmp(json_rx_name, "RelayI") == 0)
         {
            g_c_ind = (char)json_rx_int;
            set_ind(g_c_ind);
            eeprom_write(EEPROM_LAST_IND, g_c_ind);
            quick_swr();
            if (g_b_Loss_mode == 0) // else ANT and EFF are on those rows
               lcd_ind();
         }
         else if (strcmp(json_rx_name, "RelayC") == 0)
         {
            g_c_cap = (char)json_rx_int & 0x7f;
            g_c_SW = (json_rx_int & 0x80) != 0; // bit 7: C on the input side
            set_cap(g_c_cap);
            set_sw(g_c_SW);
            eeprom_write(EEPROM_LAST_CAP, g_c_cap);
            eeprom_write(EEPROM_LAST_SW, g_c_SW);
            quick_swr();
            if (g_b_Loss_mode == 0)
               lcd_ind();
         }
         else
            send_error();
         break;

      case JRR_STR:
         if (strcmp(json_rx_name, "Dump") == 0)
            dump_eeprom();
         else if (strcmp(json_rx_name, "ESP32") != 0) // for an ESP32 bridge: none here
            send_error();
         break;

      default:
         break;
      }
   }
}

// While a tune waits for carrier: answer Status and Raw, stop the tune on
// Reset or Tune (true), and answer anything else with a Busy event once
// the message has ended
char serial_during_tune(void)
{
   static char busy = 0;
   char c;

   while ((c = uart_getc()) != 0)
   {
      char letter = line_letter(c) | 0x20; // a letter alone on its line
      if (c == '{')
         busy = 0;
      json_rx_result_t result = json_rx_process(c);
      switch (result)
      {
      case JRR_NOT_JSON:
         if (letter == 's')
         {
            new_state = true;
            send_state();
         }
         else if (letter == 'r' | letter == 't')
            return 1;
         break;

      case JRR_BOOL:
         if (strcmp(json_rx_name, "Reset") == 0 || strcmp(json_rx_name, "Tune") == 0)
         {
            busy = 0;
            return 1;
         }
         if (strcmp(json_rx_name, "Status") == 0)
         {
            new_state = true;
            send_state();
         }
         else if (strcmp(json_rx_name, "Raw") == 0)
            send_raw();
         else
            busy = 1;
         break;

      case JRR_STR:
         if (strcmp(json_rx_name, "ESP32") == 0)
            break; // for an ESP32 bridge: none here
         // fall through
      case JRR_INT:
         busy = 1;
         if (c != '}') // a number can end the message without a JRR_END
            break;
         // fall through
      case JRR_END:
         if (busy)
            json_event("Busy");
         busy = 0;
         break;

      default:
         break;
      }
   }
   return 0;
}

//***************** Tuner actions and display *****************

void show_reset()
{
   atu_reset();
   g_c_SW = 1;
   set_sw(g_c_SW);
   eeprom_write(EEPROM_LAST_CAP, 0);
   eeprom_write(EEPROM_LAST_IND, 0);
   eeprom_write(EEPROM_LAST_SW, 1);
   eeprom_write(EEPROM_LAST_SWR_H, 0);
   eeprom_write(EEPROM_LAST_SWR_L, 0);
   lcd_ind();
   g_b_Loss_mode = 0;
   g_i_SWR = 0;
   g_i_PWR = 0;
   g_i_SWR_fixed_old = 0;
   json_event("Reset");
   if (e_c_ledtype != 0)
   {
      show_swr_text("RESET   ");
      Delay_ms(600);
      show_swr_text("SWR=0.00");
   }
   else
      led_swr(0);
   g_i_SWR_old = DEFAULT_INITIAL_OLD_VALUE;
   g_i_Power_old = DEFAULT_INITIAL_OLD_VALUE;
   lcd_pwr();
   return;
}

// "Tune" and "Tuned" events stand in for the Tx_req signal: transmit a
// carrier from the first until the second.
void tune_btn_push()
{
   send_state(); // what made an auto tune start: the high SWR, before the tune
   json_event("Tune");
   Delay_ms(250); // time for the transmitter to start
   disp(2, 16 + 12 * 4, 1, 4, "TUNE", 4);
   led_swr(0);
   g_b_tune_given_up = 0;
   tune();
   if (g_b_tune_given_up)
      show_reset(); // relays, display and the Reset event, as before
   if (e_c_ledtype == 0)
      led_swr(g_i_SWR);
   else if (g_b_Loss_mode == 0 | e_c_b_Loss_ind == 0)
      lcd_ind();
   eeprom_write(EEPROM_LAST_CAP, g_c_cap);
   eeprom_write(EEPROM_LAST_IND, g_c_ind);
   eeprom_write(EEPROM_LAST_SW, g_c_SW);
   eeprom_write(EEPROM_LAST_SWR_H, (char)(g_i_swr_a / 256));
   eeprom_write(EEPROM_LAST_SWR_L, (char)(g_i_swr_a % 256));
   g_i_SWR_old = DEFAULT_INITIAL_OLD_VALUE;
   g_i_Power_old = DEFAULT_INITIAL_OLD_VALUE;
   lcd_pwr();
   g_i_SWR_fixed_old = g_i_SWR;
   json_event("Tuned");
   return;
}

void lcd_prep()
{
   if (g_b_lcd_prep_short == 0)
   {
      json_event("Startup");
      json_start();
      json_str("Board", "ATU-100_EXT");
      json_str("Credit", "N7DDC");
      json_str("FW", "3.2");
      json_str("Build", FW_BUILD);
      json_end();
      if (big_oled())
      { // 128*64 OLED
         led_wr_str(0, 22, "ATU-100", 7);
         led_wr_str(2, 6, "EXT board", 9);
         led_wr_str(4, 16, "by N7DDC", 8);
         led_wr_str(6, 4, "FW ver 3.2", 10);
         Delay_ms(1100);
         led_wr_str(0, 16, "        ", 8);
         led_wr_str(2, 4, "          ", 10);
         led_wr_str(4, 16, "        ", 8);
         led_wr_str(6, 4, "          ", 10);
      }
      else if (e_c_ledtype != 0)
      { // 1602 LCD & 128*32 OLED
         led_wr_str(0, 4, "ATU-100", 7);
         led_wr_str(1, 3, "EXT board", 9);
         Delay_ms(1200);
         led_wr_str(0, 4, "by N7DDC", 8);
         led_wr_str(1, 3, "FW ver 3.2", 10);
         Delay_ms(1100);
         led_wr_str(0, 4, "        ", 8);
         led_wr_str(1, 3, "          ", 10);
      }
   }
   Delay_ms(150);
   disp(0, 16, 0, 0, e_c_b_P_High == 1 ? "PWR=  0W" : "PWR=0.0W", 8);
   show_swr_text("SWR=0.00");
   if (g_b_Auto_mode)
      show_mode();
   lcd_ind();
   new_state = true;
   return;
}

void lcd_swr(int swr)
{
   g_i_SWR_fixed = swr;
   if (swr != g_i_SWR_old)
   {
      g_i_SWR_old = swr;
      new_state = true;
      if (e_c_ledtype == 0)
         led_swr(swr); // real-time 2-colors led work
      else
      {
         g_work_str_2[0] = '0';
         g_work_str_2[1] = '.';
         g_work_str_2[2] = '0';
         g_work_str_2[3] = '0';
         if (swr != 0)
         {
            IntToStr(swr, g_work_str);
            g_work_str_2[0] = g_work_str[3];
            g_work_str_2[2] = g_work_str[4];
            g_work_str_2[3] = g_work_str[5];
         }
         disp(2, 16 + 4 * 12, 1, 4, g_work_str_2, 4);
      }
   }
   return;
}

// Power as 4 characters in g_work_str_2: "1.5W", " 15W", "150W" or "1500"
static void fmt_pwr(int power)
{
   if (e_c_b_P_High == 0)
   {
      if (power >= 100)
      {               // > 10 W
         power += 5;  // rounding to 1 W
         IntToStr(power, g_work_str);
         g_work_str_2[0] = g_work_str[2];
         g_work_str_2[1] = g_work_str[3];
         g_work_str_2[2] = g_work_str[4];
      }
      else
      {
         IntToStr(power, g_work_str);
         g_work_str_2[0] = (g_work_str[4] != ' ') ? g_work_str[4] : '0';
         g_work_str_2[1] = '.';
         g_work_str_2[2] = (g_work_str[5] != ' ') ? g_work_str[5] : '0';
      }
      g_work_str_2[3] = 'W';
   }
   else
   { // High power
      if (power > 9999)
      {
         power = 9999;
         g_b_Overload = 1;
      }
      IntToStr(power, g_work_str);
      if (power < 999)
      { // 0 - 999 Watt
         g_work_str_2[0] = g_work_str[3];
         g_work_str_2[1] = g_work_str[4];
         g_work_str_2[2] = g_work_str[5];
         g_work_str_2[3] = 'W';
      }
      else
         memcpy(g_work_str_2, &g_work_str[2], 4);
   }
}

void show_pwr(int parm_Power, int parm_SWR)
{
   //
   if (g_b_Test_mode == 0 & e_c_b_Loss_ind == 1 & parm_Power >= 10)
   {
      if (g_b_Loss_mode == 0)
      { // prepare
         if (big_oled())
         { // 128*64 OLED
            led_wr_str(4, 16, e_c_b_P_High == 1 ? "ANT=  0W" : "ANT=0.0W", 8);
            led_wr_str(6, 16, "EFF=  0%", 8);
         }
         else if (e_c_ledtype == 2 | e_c_ledtype == 3)
         { // 128*32 OLED
            led_wr_str(0, 9, e_c_b_P_High == 1 ? "ANT=  0W" : "ANT=0.0W", 8);
            led_wr_str(1, 9, "EFF=  0%", 8);
         }
         else if (e_c_ledtype == 1)
         { // 1602 LCD
            led_wr_str(0, 9, e_c_b_P_High == 1 ? "AN=  0W" : "AN=0.0W", 7);
            led_wr_str(1, 9, "EFF= 0%", 7);
         }
      }
      g_b_Loss_mode = 1;
   }
   else
   {
      if (g_b_Loss_mode == 1)
         lcd_ind();
      g_b_Loss_mode = 0;
      g_i_Efficency = 0;
   }
   // Antenna power and efficiency depend on the SWR too: redo them when
   // either changes (N7DDC's code only looked at the power)
   if (parm_Power != g_i_Power_old | (g_b_Loss_mode == 1 & parm_SWR != g_i_loss_swr_old))
   {
      g_i_Power_old = parm_Power;
      g_i_loss_swr_old = parm_SWR;
      g_i_Power_tx = parm_Power;
      g_i_Power_report = parm_Power;
      new_state = true;
      //
      fmt_pwr(parm_Power);
      disp(0, 16 + 4 * 12, 0, 4, g_work_str_2, 4);
      //
      CLRWDT();
      //  Loss indication
      if (g_b_Loss_mode == 1)
      {
         if (g_c_ind == 0 & g_c_cap == 0)
            g_i_swr_a = parm_SWR;
         if (g_i_swr_a < 100)
            g_i_swr_a = 100; // none saved yet: 0 would divide by zero
         m_in(0, g_i_swr_a); // feeder loss and SWR loss
         m_in(1, e_c_tenths_Fid_loss);
         m_in(2, parm_SWR);
         m_in(3, parm_Power);
         m_calc(p_loss);
         g_i_Efficency = t_out[1];
         if (g_i_Efficency >= 100)
            g_i_Efficency = 99;
         g_i_Power_report = t_out[0];
         //
         fmt_pwr(t_out[0]);
         if (big_oled())
            led_wr_str(4, 16 + 4 * 12, g_work_str_2, 4); // 128*64 OLED
         else if (e_c_ledtype == 2 | e_c_ledtype == 3)
            led_wr_str(0, 13, g_work_str_2, 4); // 128*32
         else if (e_c_ledtype != 0)
            led_wr_str(0, 12, g_work_str_2, 4); // 1602
         //
         IntToStr(g_i_Efficency, g_work_str);
         if (big_oled())
            led_wr_str(6, 16 + 5 * 12, &g_work_str[4], 2);
         else if (e_c_ledtype == 2 | e_c_ledtype == 3)
            led_wr_str(1, 14, &g_work_str[4], 2);
         else if (e_c_ledtype == 1)
            led_wr_str(1, 13, &g_work_str[4], 2);
      }
   }
   return;
}

// HighSWR event as soon as one reading with power on it is above the
// limit; again only after the SWR has dropped 0.1 below it or the power
// has gone.
// Samples (one each 3 ms or so in lcd_pwr) the alarm condition must stay
// clear before an alarm can be sent again: about 2 s, so a gap between CW
// dits or SSB syllables does not send a new event each time
#define ALARM_REARM 500

static void swr_alarm(void)
{
   static int clear = 0;
   if (e_i_swr_alarm == 0)
      return;
   if (g_i_PWR >= 10 && g_i_SWR >= e_i_swr_alarm)
   {
      clear = 0;
      if (!g_b_swr_alarm)
      {
         g_b_swr_alarm = 1;
         json_start(); // with the reading that set it off
         json_str("Event", "HighSWR");
         json_int("SWR", g_i_SWR, 2);
         json_end();
      }
   }
   else if (g_i_PWR < 10 || g_i_SWR < e_i_swr_alarm - 10)
   {
      if (g_b_swr_alarm && ++clear >= ALARM_REARM)
         g_b_swr_alarm = 0;
   }
}

// One reading straight after the relays changed, so the status that
// follows carries the SWR of the new relays, not the old ones. No power
// in that moment (a gap in speech) gives 0.00 until the next measurement.
static void quick_swr(void)
{
   int swr;
   Delay_ms(e_i_ms_Rel_Del); // the relays settle
   get_pwr();
   swr = g_i_PWR >= 10 ? g_i_SWR : 0;
   lcd_swr(swr);
   if (swr != 0)
      show_pwr(g_i_Power, swr); // antenna power and efficiency for the new SWR
}

// More power than the board or the detector winding is good for: the
// board is rated 100 W; a 1:5 tandem match tops out near 37 W
static void power_alarm(void)
{
   static int clear = 0;
   if (e_i_power_alarm == 0)
      return;
   if (g_i_PWR >= e_i_power_alarm)
   {
      clear = 0;
      if (!g_b_power_alarm)
      {
         g_b_power_alarm = 1;
         json_start();
         json_str("Event", "HighPower");
         json_int("Power", g_i_PWR, 1);
         json_end();
      }
   }
   else if (g_i_PWR < e_i_power_alarm - e_i_power_alarm / 10)
   {
      if (g_b_power_alarm && ++clear >= ALARM_REARM)
         g_b_power_alarm = 0;
   }
}

void lcd_pwr()
{
   int p = 0;
   char peak_cnt;
   int delta = e_i_tenths_SWR_Auto_delta - 100;
   int SWR_fixed = 0;
   g_i_PWR = 0;
   CLRWDT();
   //
   if (g_b_display_onoff == 0 & e_c_b_Relay_off == 1)
   {
      get_pwr();
      if (g_i_PWR >= 10)
         wake();
      return;
   }
   // peak detector
   for (peak_cnt = 0; peak_cnt < 120; peak_cnt++)
   {
      if (button_abort() || uart_rx_message())
         return; // a button, or a serial message to answer straight away
      get_pwr();
      swr_alarm();
      power_alarm();
      if (g_i_PWR > p)
      {
         p = g_i_PWR;
         SWR_fixed = g_i_SWR;
      }
      Delay_ms(3);
   }
   CLRWDT();
   if (p >= 100)
   {
      p = (p + 5) / 10;
      p *= 10;
   } // round to 1 W if more then 100 W
   g_i_Power = p;
   if (g_i_Power < 10)
      SWR_fixed = 0;
   lcd_swr(SWR_fixed);
   if (g_i_Power >= 10)
      wake(); // RF turns the display back on and restarts its off timer
   //
   if (g_b_Auto_mode && SWR_fixed >= e_i_tenths_SWR_Auto_delta)
   { // auto: the SWR moved by more than delta either way, or first time
      int change = SWR_fixed - g_i_SWR_fixed_old;
      if (change > delta || -change > delta || g_i_SWR_fixed_old == 999)
         g_b_Soft_tune = 1;
   }
   //
   if (button_abort())
      return; // Fast return if button pressed
   //
   show_pwr(g_i_Power, SWR_fixed);
   //
   if (button_abort())
      return;
   CLRWDT();
   if (g_b_Overload == 1)
   {
      json_event("Overload");
      show_swr_text("        ");
      Delay_ms(100);
      for (peak_cnt = 0; peak_cnt < 3; peak_cnt++)
      {
         show_swr_text("OVERLOAD");
         Delay_ms(500);
         show_swr_text("        ");
         Delay_ms(peak_cnt == 2 ? 100 : 300);
      }
      show_swr_text("SWR=    ");
      g_i_SWR_old = DEFAULT_INITIAL_OLD_VALUE;
      lcd_swr(SWR_fixed);
   }
   return;
}

void lcd_ind()
{
   char l_row;
   int l_work_int;
   CLRWDT();
   new_state = true;
   l_work_int = relay_sum(g_c_ind, e_i_Ind);
   if (l_work_int > 9999)
   {                  // more then 9999 nH
      l_work_int += 50; // round
      IntToStr(l_work_int, g_work_str);
      g_work_str_2[0] = g_work_str[1];
      g_work_str_2[1] = g_work_str[2];
      g_work_str_2[2] = '.';
      g_work_str_2[3] = g_work_str[3];
   }
   else
   {
      IntToStr(l_work_int, g_work_str);
      g_work_str_2[0] = (g_work_str[2] != ' ') ? g_work_str[2] : '0';
      g_work_str_2[1] = '.';
      g_work_str_2[2] = (g_work_str[3] != ' ') ? g_work_str[3] : '0';
      g_work_str_2[3] = (g_work_str[4] != ' ') ? g_work_str[4] : '0';
   }
   // L above C when the capacitors are on the output side
   if (big_oled())
   { // 128*64 OLED
      l_row = (g_c_SW == 1) ? 4 : 6;
      led_wr_str(l_row, 16, "L=", 2);
      led_wr_str(l_row, 16 + 6 * 12, "uH", 2);
      led_wr_str(l_row, 16 + 2 * 12, g_work_str_2, 4);
   }
   else if (e_c_ledtype != 0)
   { // 128*32 OLED and 1602 LCD
      l_row = (g_c_SW == 1) ? 0 : 1;
      led_wr_str(l_row, 9, "L=", 2);
      led_wr_str(l_row, 15, "uH", (e_c_ledtype == 1) ? 1 : 2);
      led_wr_str(l_row, 11, g_work_str_2, 4);
   }
   CLRWDT();
   l_work_int = relay_sum(g_c_cap, e_i_Cap);
   IntToStr(l_work_int, g_work_str);
   memcpy(g_work_str_2, &g_work_str[2], 4);
   //
   if (big_oled())
   { // 128*64 OLED
      l_row = (g_c_SW == 1) ? 6 : 4;
      led_wr_str(l_row, 16, "C=", 2);
      led_wr_str(l_row, 16 + 6 * 12, "pF", 2);
      led_wr_str(l_row, 16 + 2 * 12, g_work_str_2, 4);
   }
   else if (e_c_ledtype != 0)
   { // 128*32 OLED and 1602 LCD
      l_row = (g_c_SW == 1) ? 1 : 0;
      led_wr_str(l_row, 9, "C=", 2);
      led_wr_str(l_row, 15, "pF", (e_c_ledtype == 1) ? 1 : 2);
      led_wr_str(l_row, 11, g_work_str_2, 4);
   }
   CLRWDT();
   return;
}

void Test_init(void)
{                             // g_b_Test_mode mode
   disp(0, 10, 0, 3, "TEST MODE", 9);
   Delay_ms(2000);
   disp(0, 10, 0, 3, "         ", 9);
   atu_reset();
   g_c_SW = 1; // C to OUT
   set_sw(g_c_SW);
   eeprom_write(EEPROM_LAST_CAP, g_c_cap);
   eeprom_write(EEPROM_LAST_IND, g_c_ind);
   eeprom_write(EEPROM_LAST_SW, g_c_SW);
   //
   disp(0, 16 + 12 * 8, 0, 8, "l", 1);
   //
   g_b_lcd_prep_short = 1;
   lcd_prep();
   return;
}

// Two BCD bytes from eeprom: 0x01 0x50 = 150
static int bcd_pair(char addr)
{
   return Bcd2Dec(eeprom_read(addr)) * 100 + Bcd2Dec(eeprom_read(addr + 1));
}

void cells_init(void)
{
   char i;
   // Cells init
   CLRWDT();
   e_c_ledtype = eeprom_read(EEPROM_DISPLAY_TYPE); // e_c_ledtype of display
   g_b_Auto_mode = eeprom_read(EEPROM_AUTOMATIC_MODE) == 1;
   e_i_ms_Rel_Del = Bcd2Dec(eeprom_read(EEPROM_TIMEOUT_TIME));            // Relay's Delay
   e_i_tenths_SWR_Auto_delta = Bcd2Dec(eeprom_read(EEPROM_SWR_THRESHOLD)) * 10;    // e_i_tenths_SWR_Auto_delta
   e_i_watts_min_for_start = Bcd2Dec(eeprom_read(EEPROM_MIN_POWER)) * 10; // P_min_for_start
   e_i_watts_max_for_start = Bcd2Dec(eeprom_read(EEPROM_MAX_POWER)) * 10; // P_max_for_start
   // 7  - shift down
   // 8 - shift left
   e_i_tenths_init_max_swr = Bcd2Dec(eeprom_read(EEPROM_MAX_INIT_SWR)) * 10; // Max g_i_SWR
   e_c_num_L_q = eeprom_read(EEPROM_NUMBER_INDS);
   e_c_b_L_linear = eeprom_read(EEPROM_IND_LINEAR_PITCH);
   e_c_num_C_q = eeprom_read(EEPROM_NUMBER_CAPS);
   e_c_b_C_linear = eeprom_read(EEPROM_CAP_LINEAR_PITCH);
   e_c_b_D_correction = eeprom_read(EEPROM_ENABLE_NONLINEAR_DIODE);
   e_c_b_L_invert = eeprom_read(EEPROM_INVERSE_INDUCTANCE_RELAY);
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
   //
   CLRWDT();
   for (i = 0; i < 7; i++)
   {
      e_i_Ind[i] = bcd_pair(16 + 2 * i); // cells 0x10 - 0x1d
      e_i_Cap[i] = bcd_pair(32 + 2 * i); // cells 0x20 - 0x2d
   }
   //
   e_c_b_P_High = eeprom_read(EEPROM_POWER_MEASURE_LEVEL);              // High power
   e_c_K_Mult = Bcd2Dec(eeprom_read(EEPROM_TANDEM_MATCH));     // Tandem Match rate
   e_c_b_Dysp_delay = Bcd2Dec(eeprom_read(EEPROM_DISPLAY_OFF_TIMER)); // Dysplay ON delay

   c_int_dysp_delay_time = e_c_b_Dysp_delay * 23 / 10;

   e_c_b_Loss_ind = eeprom_read(EEPROM_ADDITIONAL_INDICATION);
   e_c_tenths_Fid_loss = Bcd2Dec(eeprom_read(EEPROM_FEEDER_LOSS));
   e_c_b_Relay_off = Bcd2Dec(eeprom_read(EEPROM_DISABLE_RELAYS));
   i = eeprom_read(EEPROM_SWR_ALARM);
   e_i_swr_alarm = (i == 0xff) ? 300 : Bcd2Dec(i) * 10; // unset: 3.0
   // HighPower: the board's 100 W, or less for a smaller winding (power
   // goes with the turns ratio squared: 1.4 W x ratio^2, 35 W at 1:5).
   // Off in high power mode, where the board was built for more.
   if (e_c_b_P_High)
      e_i_power_alarm = 0;
   else
   {
      if (e_c_K_Mult >= 9) // 1.4 x 81 is past 100 W already
         e_i_power_alarm = 1000; // tenths of W
      else
         e_i_power_alarm = 14 * e_c_K_Mult * e_c_K_Mult;
   }
   CLRWDT();
   return;
}

void show_loss(void)
{
   IntToStr(e_c_tenths_Fid_loss, g_work_str);
   g_work_str_2[0] = (e_c_tenths_Fid_loss >= 10) ? g_work_str[4] : '0';
   g_work_str_2[1] = '.';
   g_work_str_2[2] = g_work_str[5];
   disp(4, 6, 1, 0, g_work_str_2, 3);
   return;
}

//
