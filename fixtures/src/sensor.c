#include "app.h"

/* A four-sample moving average and seeded ADC simulator replace board I/O. */
static unsigned filter_history[4] = {512, 512, 512, 512};
static unsigned filter_index;
static unsigned sensor_state;
static volatile unsigned last_raw_sample;

void sensor_init(unsigned seed)
{
    const DeviceConfig *config = config_get();
    sensor_state = seed;
    filter_index = 0;

    for (unsigned index = 0; index < 4; index++) {
        filter_history[index] = config->alarm_threshold >> 1;
    }
}

static unsigned sensor_read_raw(unsigned tick)
{
    sensor_state = sensor_state * 1664525u + 1013904223u + tick;
    last_raw_sample = (sensor_state >> 16) & 1023;
    return last_raw_sample;
}

unsigned sensor_sample(unsigned tick)
{
    unsigned total = 0;
    filter_history[filter_index] = sensor_read_raw(tick);
    filter_index = (filter_index + 1) & 3;

    for (unsigned index = 0; index < 4; index++) {
        total += filter_history[index];
    }

    return total >> 2;
}
