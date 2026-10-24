#ifndef json_h
#define json_h

#include <stdbool.h>
#include <stdint.h>


#define JSON_MAX_STR    16

typedef enum {
    JRR_NO_DATA,
    JRR_NOT_JSON,
    JRR_BOOL,
    JRR_INT,
    JRR_STR,
    JRR_END
} json_rx_result_t;

extern char json_rx_name[JSON_MAX_STR];
extern bool json_rx_bool;
extern int  json_rx_int;
extern char json_rx_str[JSON_MAX_STR];


// JSON from ATU-100

void json_start(void);
void json_end(void);
void json_str(const char *name, const char *value);
void json_bool(const char *name, bool value);
void json_int(const char *name, unsigned int value, uint8_t dp);
void json_event(const char *event);

// JSON to ATU-100

json_rx_result_t json_rx_process(char rx);

#endif
