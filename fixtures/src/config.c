#include "app.h"

static const DeviceConfig app_config = {
    .sample_period_ticks = 4,
    .report_interval_ticks = 8,
    .alarm_threshold = 640,
    .checksum_seed = 0x13579bdf,
};

const unsigned baudrate_table[] = {9600, 19200, 38400, 115200};

static const unsigned profile_margin[] = {0, 16, 32, 64};

const DeviceConfig *config_get(void)
{
    return &app_config;
}

unsigned config_baudrate(void)
{
    return baudrate_table[2];
}

unsigned config_alarm_threshold(unsigned profile)
{
    return app_config.alarm_threshold + profile_margin[profile & 3];
}

unsigned config_checksum_seed(void)
{
    return app_config.checksum_seed;
}
