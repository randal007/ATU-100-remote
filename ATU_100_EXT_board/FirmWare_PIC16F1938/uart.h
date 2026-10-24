#ifndef uart_h
#define uart_h

#include "cross_compiler.h"
#include <stdint.h>

void uart_init(void);
void uart_putc(char c);
void uart_str(const char *str);
bool uart_rx_message(void);
char uart_getc(void); // 0 when nothing has arrived

#endif
