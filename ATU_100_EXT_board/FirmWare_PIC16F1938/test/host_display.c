/*  Display stand-in: keeps the text the firmware writes in a character
    grid instead of driving I2C, so the tests can print and check it. */
#include <stdio.h>
#include <string.h>
#include "../oled_control.h"

static char grid[4][23];
static char pixels[4][128]; // 128*64 OLED: character starting at each pixel column, 1 = covered
static int type, on = 1;

void Soft_I2C_Init(void)
{
    type = eeprom_read(EEPROM_DISPLAY_TYPE);
}

void led_init(void)
{
    memset(grid, ' ', sizeof grid);
    memset(pixels, 0, sizeof pixels);
    for (int r = 0; r < 4; r++)
        grid[r][22] = 0;
}

void dysp_on(void) { on = 1; }
void dysp_off(void) { on = 0; }

// col is the row (page on an OLED), pos the column (pixels on a 128*64 OLED)
void led_wr_str(char col, char pos, char *str, char leng)
{
    for (int i = 0; i < leng; i++) {
        char c = str[i] ? str[i] : ' ';
        if (type == 4 || type == 5) { // 12 pixel wide characters on page pairs
            int x = pos + 12 * i;
            for (int p = x; p < x + 12 && p < 128 && col / 2 < 4; p++)
                pixels[col / 2][p] = (p == x) ? c : 1;
        } else if (col < 4 && pos + i < 22) {
            grid[(int)col][pos + i] = c;
        }
    }
}

// Characters on a 128*64 OLED row, one text column per 6 pixels
static void render_big(void)
{
    for (int r = 0; r < 4; r++) {
        memset(grid[r], ' ', 22);
        for (int p = 0; p < 128; p++)
            if (pixels[r][p] > 1 && p / 6 < 22)
                grid[r][p / 6] = pixels[r][p];
    }
}

void host_show_display(const char *title)
{
    if (type >= 4)
        render_big();
    int rows = (type >= 4) ? 4 : 2;
    printf("    +----------------------+ %s%s\n", title, on ? "" : " (display off)");
    for (int r = 0; r < rows; r++)
        printf("    |%s|\n", grid[r]);
    printf("    +----------------------+\n");
}

const char *host_display_row(int r)
{
    if (type >= 4)
        render_big();
    return grid[r];
}
