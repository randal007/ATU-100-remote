#include "json.h"
#include "uart.h"

/*  Messages from the tuner are one line each: {"Name":value,...}\n
    Messages to the tuner may contain any whitespace, e.g. {"Tune": true} */

char json_rx_name[JSON_MAX_STR];
bool json_rx_bool;
int  json_rx_int;
char json_rx_str[JSON_MAX_STR];

typedef enum {
    JRS_START_WAIT,
    JRS_NAME_WAIT,
    JRS_NAME,
    JRS_VALUE_WAIT,
    JRS_INT,
    JRS_STR
} json_rx_state_t;

static json_rx_state_t json_rx_state = JRS_START_WAIT;
static uint8_t json_rx_pos;
static bool have_var;

void json_start(void)
{
    have_var = false;
    uart_putc('{');
}


void json_end(void)
{
    uart_str("}\n");
}


static void json_name(const char *name)
{
    if (have_var)
        uart_putc(',');
    uart_putc('"');
    uart_str(name);
    uart_str("\":");
    have_var = true;
}


void json_str(const char *name, const char *value)
{
    json_name(name);
    uart_putc('"');
    uart_str(value);
    uart_putc('"');
}


void json_bool(const char *name, bool value)
{
    json_name(name);
    uart_str(value ? "true" : "false");
}


// value with dp digits after the decimal point: (150, 2) is 1.50
void json_int(const char *name, unsigned int value, uint8_t dp)
{
    char str[8];
    char *p = &str[7];
    uint8_t n = 0;

    json_name(name);
    *p = 0;
    do {
        if (n == dp && dp)
            *--p = '.';
        *--p = (char)('0' + value % 10);
        value /= 10;
        n++;
    } while (value || n <= dp);
    uart_str(p);
}


void json_event(const char *event)
{
    json_start();
    json_str("Event", event);
    json_end();
}


static void store(char *str, char rx)
{
    if (json_rx_pos < JSON_MAX_STR - 1) {
        str[json_rx_pos++] = rx;
        str[json_rx_pos] = 0;
    }
}


json_rx_result_t json_rx_process(char rx)
{
    // One message per line. A { or a line end inside a message means
    // characters were lost: drop the rest of it rather than act on half
    if (json_rx_state != JRS_START_WAIT && (rx == '{' || rx == '\n' || rx == '\r')) {
        json_rx_state = (rx == '{') ? JRS_NAME_WAIT : JRS_START_WAIT;
        return JRR_NO_DATA;
    }
    switch (json_rx_state) {
    case JRS_START_WAIT:
        if (rx != '{')
            return JRR_NOT_JSON;
        json_rx_state = JRS_NAME_WAIT;
        break;

    case JRS_NAME_WAIT:
        if (rx == '}') {
            json_rx_state = JRS_START_WAIT;
            return JRR_END;
        }
        if (rx == '"') {
            json_rx_pos = 0;
            json_rx_name[0] = 0;
            json_rx_state = JRS_NAME;
        }
        break;

    case JRS_NAME:
        if (rx != '"') {
            store(json_rx_name, rx);
        } else if (json_rx_name[0] == 'x' && json_rx_pos == 1) {
            RESET(); // {"x": ...} restarts the board
        } else {
            json_rx_state = JRS_VALUE_WAIT;
        }
        break;

    case JRS_VALUE_WAIT:
        if (rx == '"') {
            json_rx_pos = 0;
            json_rx_str[0] = 0;
            json_rx_state = JRS_STR;
        } else if (rx == 't' || rx == 'f') {
            json_rx_bool = (rx == 't');
            json_rx_state = JRS_NAME_WAIT;
            return JRR_BOOL;
        } else if (rx >= '0' && rx <= '9') {
            json_rx_int = rx - '0';
            json_rx_state = JRS_INT;
        }
        break;

    case JRS_INT:
        if (rx >= '0' && rx <= '9') {
            json_rx_int = json_rx_int * 10 + rx - '0';
            break;
        }
        // The character after a number may be the closing brace
        json_rx_state = (rx == '}') ? JRS_START_WAIT : JRS_NAME_WAIT;
        return JRR_INT;

    case JRS_STR:
        if (rx == '"') {
            json_rx_state = JRS_NAME_WAIT;
            return JRR_STR;
        }
        store(json_rx_str, rx);
        break;
    }

    return JRR_NO_DATA;
}
